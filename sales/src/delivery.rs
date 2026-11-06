//! Delivery quantities are reserved from draft onward. All callers hold the
//! company write lock before preparing a delivery or changing its source order.
use crate::{
    prepare::{decimal, integer, required, text, Prepared},
    store, Record,
};
use bilibi_agent_api::{Error, OperationClient, Result};
use bilibi_tenancy::Scope;
use rust_decimal::Decimal;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    str::FromStr,
};

pub(crate) async fn has_active(
    c: &dyn OperationClient,
    s: &Scope,
    co: i64,
    order: i64,
) -> Result<bool> {
    Ok(c.query_opt("SELECT id FROM commerce_sales_documents WHERE tenant_id=$1 AND company_id=$2 AND kind='delivery' AND (facts->>'order_id')::bigint=$3 AND status<>'cancelled' LIMIT 1", &[&s.tenant(), &co, &order]).await?.is_some())
}
async fn quantities(
    c: &dyn OperationClient,
    s: &Scope,
    co: i64,
    order: i64,
) -> Result<BTreeMap<i64, (Decimal, Decimal)>> {
    let rows = c.query("SELECT (line->>'order_line_number')::bigint, sum((line->>'quantity')::numeric)::text, COALESCE(sum((line->>'quantity')::numeric) FILTER (WHERE status='delivered'),0)::text FROM commerce_sales_documents CROSS JOIN LATERAL jsonb_array_elements(facts->'lines') line WHERE tenant_id=$1 AND company_id=$2 AND kind='delivery' AND (facts->>'order_id')::bigint=$3 AND status<>'cancelled' GROUP BY 1", &[&s.tenant(), &co, &order]).await?;
    rows.iter()
        .map(|r| {
            let reserved = Decimal::from_str(
                &r.try_get::<_, String>(1)
                    .map_err(bilibi_database::Error::from)?,
            )
            .map_err(|_| bilibi_database::Error::Internal)?;
            let delivered = Decimal::from_str(
                &r.try_get::<_, String>(2)
                    .map_err(bilibi_database::Error::from)?,
            )
            .map_err(|_| bilibi_database::Error::Internal)?;
            Ok((
                r.try_get(0).map_err(bilibi_database::Error::from)?,
                (reserved, delivered),
            ))
        })
        .collect()
}
pub(crate) async fn progress(
    c: &dyn OperationClient,
    s: &Scope,
    co: i64,
    order: &mut Record,
) -> bilibi_database::Result<()> {
    let used = quantities(c, s, co, order.id)
        .await
        .map_err(|_| bilibi_database::Error::Internal)?;
    if let Some(lines) = order.facts["lines"].as_array_mut() {
        for line in lines {
            let number = line["line_number"]
                .as_i64()
                .ok_or(bilibi_database::Error::Internal)?;
            let quantity = Decimal::from_str(text(line, "quantity"))
                .map_err(|_| bilibi_database::Error::Internal)?;
            let (reserved, delivered) = used.get(&number).copied().unwrap_or_default();
            line["delivered"] = json!(delivered.to_string());
            line["remaining"] = json!((quantity - delivered).to_string());
            line["available"] = json!((quantity - reserved).to_string());
        }
    }
    Ok(())
}
pub(crate) async fn prepare(
    c: &dyn OperationClient,
    s: &Scope,
    v: &Value,
    mut p: Prepared,
) -> Result<Prepared> {
    if let Some(old) = &p.old {
        let to = required(v, "status", 20)?;
        if !matches!(
            (old.status.as_str(), to.as_str()),
            ("draft", "shipped" | "cancelled") | ("shipped", "delivered" | "cancelled")
        ) {
            return Err(bilibi_agent_api::invalid(
                "status",
                "This delivery cannot move to that status.",
            ));
        }
        if to != "cancelled" {
            confirmed_order(c, s, p.company, integer(&old.facts, "order_id")?).await?;
        }
        p.reason = if to == "cancelled" {
            required(v, "reason", 500)?
        } else {
            text(v, "reason").to_owned()
        };
        if p.reason.chars().count() > 500 {
            return Err(bilibi_agent_api::invalid(
                "reason",
                "Use up to 500 characters.",
            ));
        }
        p.target = to;
        return Ok(p);
    }
    let order_id = if v["order_id"].as_i64().is_some() {
        if v["order_number"].as_i64().is_some() {
            return Err(bilibi_agent_api::invalid(
                "order_id",
                "Choose an order by its ID or its number, not both.",
            ));
        }
        integer(v, "order_id")?
    } else {
        let number = integer(v, "order_number")?;
        c.query_opt("SELECT id FROM commerce_sales_documents WHERE tenant_id=$1 AND company_id=$2 AND kind='order' AND number=$3", &[&s.tenant(),&p.company,&number]).await?.ok_or(Error::NotFound)?.try_get(0).map_err(bilibi_database::Error::from)?
    };
    let order = confirmed_order(c, s, p.company, order_id).await?;
    let used = quantities(c, s, p.company, order_id).await?;
    let source = order.facts["lines"]
        .as_array()
        .ok_or(bilibi_database::Error::Internal)?;
    let selected = v.get("lines").filter(|v| !v.is_null());
    let requests = if let Some(lines) = selected {
        lines
            .as_array()
            .filter(|a| !a.is_empty() && a.len() <= 100)
            .ok_or_else(|| {
                bilibi_agent_api::invalid("lines", "Choose between 1 and 100 order lines.")
            })?
            .clone()
    } else {
        source
            .iter()
            .filter_map(|l| {
                let n = l["line_number"].as_i64()?;
                let q = Decimal::from_str(text(l, "quantity")).ok()?
                    - used.get(&n).copied().unwrap_or_default().0;
                (q > Decimal::ZERO).then(|| json!({"order_line_number":n,"quantity":q.to_string()}))
            })
            .collect()
    };
    if requests.is_empty() {
        return Err(bilibi_agent_api::invalid(
            "lines",
            "This order has no quantities available for delivery.",
        ));
    }
    let mut seen = BTreeSet::new();
    let mut lines = Vec::new();
    for request in requests {
        let n = integer(&request, "order_line_number")?;
        if !seen.insert(n) {
            return Err(bilibi_agent_api::invalid(
                "lines",
                "Choose each order line only once.",
            ));
        }
        let original = source
            .iter()
            .find(|l| l["line_number"].as_i64() == Some(n))
            .ok_or_else(|| {
                bilibi_agent_api::invalid("lines", "Choose a line from this sales order.")
            })?;
        let quantity = decimal(&request, "quantity", true)?;
        let ordered = Decimal::from_str(text(original, "quantity"))
            .map_err(|_| bilibi_database::Error::Internal)?;
        let available = ordered - used.get(&n).copied().unwrap_or_default().0;
        if quantity > available {
            return Err(bilibi_agent_api::invalid(
                "lines",
                format!("Order line {n} has {available} available for delivery."),
            ));
        }
        lines.push(json!({"line_number":lines.len()+1,"order_line_number":n,"product_id":original["product_id"],"product_code":original["product_code"],"description":original["description"],"unit":original["unit"],"ordered_quantity":original["quantity"],"quantity":quantity.to_string()}));
    }
    p.facts = json!({"order_id":order.id,"order_number":order.code,"order_revision":order.revision,"customer_kind":order.facts["customer_kind"],"customer_id":order.facts["customer_id"],"customer_code":order.facts["customer_code"],"reference":text(v,"reference"),"lines":lines});
    if text(v, "reference").chars().count() > 200 {
        return Err(bilibi_agent_api::invalid(
            "reference",
            "Use up to 200 characters.",
        ));
    }
    Ok(p)
}
async fn confirmed_order(c: &dyn OperationClient, s: &Scope, co: i64, id: i64) -> Result<Record> {
    let order = store::get(c, s.tenant(), co, "order", id)
        .await?
        .ok_or(Error::NotFound)?;
    if order.status != "confirmed" {
        return Err(bilibi_agent_api::invalid(
            "order_id",
            "Choose a confirmed sales order.",
        ));
    }
    Ok(order)
}
