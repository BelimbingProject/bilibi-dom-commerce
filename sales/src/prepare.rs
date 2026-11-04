//! All business validation and exact arithmetic live here, shared by preview
//! and commit. Product references resolve under the company lock.
use crate::{Record, store};
use bilibi_agent_api::{Error, OperationClient, Result};
use bilibi_records::Records;
use bilibi_tenancy::{PartyProofs, Scope};
use rust_decimal::{Decimal, RoundingStrategy};
use serde_json::{Value, json};
use std::{collections::BTreeMap, str::FromStr};

pub(crate) fn invalid(field: &str, message: &str) -> Error {
    Error::InvalidInput(BTreeMap::from([(field.into(), vec![message.into()])]))
}
pub(crate) fn text<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or_default().trim()
}
pub(crate) fn integer(v: &Value, k: &str) -> Result<i64> {
    v[k].as_i64()
        .filter(|i| *i > 0)
        .ok_or_else(|| invalid(k, "Choose a valid record."))
}
pub(crate) fn required(v: &Value, k: &str, max: usize) -> Result<String> {
    let s = text(v, k);
    if s.is_empty() {
        return Err(invalid(k, &format!("Enter a {}.", k.replace('_', " "))));
    }
    if s.chars().count() > max {
        return Err(invalid(k, &format!("Use up to {max} characters.")));
    }
    if s.contains('\0') {
        return Err(invalid(k, "Remove unsupported characters."));
    }
    Ok(s.into())
}
pub(crate) fn decimal(v: &Value, k: &str, positive: bool) -> Result<Decimal> {
    let s = required(v, k, 30)?;
    let d = Decimal::from_str(&s).map_err(|_| invalid(k, "Enter a decimal number."))?;
    if d.scale() > 4
        || d < Decimal::ZERO
        || (positive && d == Decimal::ZERO)
        || d > Decimal::from(999_999_999_i64)
    {
        return Err(invalid(
            k,
            "Enter a positive quantity or a nonnegative price, with up to four decimals.",
        ));
    }
    Ok(d)
}
fn money(d: Decimal, places: u32) -> Decimal {
    let mut d = d.round_dp_with_strategy(places, RoundingStrategy::MidpointAwayFromZero);
    d.rescale(places);
    d
}
fn date(v: &Value, k: &str) -> Result<String> {
    let s = required(v, k, 10)?;
    if s.len() != 10 || jiff::civil::Date::from_str(&s).is_err() {
        return Err(invalid(k, "Enter a date as YYYY-MM-DD."));
    }
    Ok(s)
}
fn today() -> String {
    jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date()
        .to_string()
}

pub(crate) struct Prepared {
    pub company: i64,
    pub kind: String,
    pub code: String,
    pub old: Option<Record>,
    pub facts: Value,
    pub target: String,
    pub reason: String,
    pub quotation: Option<i64>,
}
impl Prepared {
    pub fn review(&self) -> Value {
        json!({"company_id":self.company,"kind":self.kind,"code":self.code,"before":self.old.as_ref().map(Record::snapshot),"facts":self.facts,"status":self.target,"reason":self.reason,"quotation_id":self.quotation})
    }
}

// Preview borrows the operation's transaction; commit additionally locks the
// enabled assignment through Core Currency, keeping a concurrent disable out.
async fn currency(
    c: &dyn OperationClient,
    scope: &Scope,
    code: &str,
    currencies: &bilibi_currency::Currencies,
    tx: Option<&bilibi_database::MutationTransaction<'_>>,
) -> Result<bilibi_currency::Currency> {
    if let Some(tx) = tx {
        return currencies
            .check(tx, scope, code)
            .await?
            .map_err(|e| invalid("currency_code", e.message()));
    }
    let sql = c.transaction().ok_or(bilibi_database::Error::Internal)?;
    let currency = currencies
        .get(sql, scope, code)
        .await?
        .ok_or_else(|| invalid("currency_code", "Choose a known currency."))?;
    if currency.withdrawn || !currency.enabled {
        return Err(invalid("currency_code", "Choose an enabled currency."));
    }
    Ok(currency)
}

#[allow(clippy::too_many_arguments)] // Services and the caller's transaction stay explicit.
pub(crate) async fn prepare(
    c: &dyn OperationClient,
    scope: &Scope,
    v: &Value,
    action: &str,
    records: &Records,
    parties: &PartyProofs,
    currencies: &bilibi_currency::Currencies,
    products: &bilibi_product::Products,
    tx: Option<&bilibi_database::MutationTransaction<'_>>,
) -> Result<Prepared> {
    let co = integer(v, "company_id")?;
    let kind = if action.starts_with("quotation") {
        "quotation"
    } else if action.starts_with("delivery") {
        "delivery"
    } else {
        "order"
    };
    let mut p = Prepared {
        company: co,
        kind: kind.into(),
        code: String::new(),
        old: None,
        facts: json!({}),
        target: "draft".into(),
        reason: String::new(),
        quotation: None,
    };
    if action.ends_with("transition") {
        let old = store::get(c, scope.tenant(), co, kind, integer(v, "id")?)
            .await?
            .ok_or(Error::NotFound)?;
        if integer(v, "revision")? != old.revision {
            return Err(bilibi_database::Error::Conflict.into());
        }
        p.code = old.code.clone();
        p.facts = old.facts.clone();
        p.old = Some(old);
    }
    if kind == "delivery" {
        return crate::delivery::prepare(c, scope, v, p).await;
    }
    if action.ends_with("transition") {
        let old = p.old.as_ref().ok_or(Error::NotFound)?;
        let to = required(v, "status", 20)?;
        let valid = match (kind, old.status.as_str(), to.as_str()) {
            ("quotation", "draft", "sent") => true,
            ("quotation", "sent", "accepted" | "rejected") => {
                text(&old.facts, "valid_until") >= today().as_str()
            }
            ("quotation", "sent", "expired") => text(&old.facts, "valid_until") < today().as_str(),
            ("order", "draft", "confirmed") | ("order", "draft" | "confirmed", "cancelled") => true,
            _ => false,
        };
        if !valid {
            return Err(invalid(
                "status",
                "This action no longer applies to the record or its expiry date.",
            ));
        }
        if to == "sent" && text(&old.facts, "valid_until") < today().as_str() {
            return Err(invalid(
                "valid_until",
                "This quotation has expired. Create a new quotation.",
            ));
        }
        if kind == "order"
            && to == "cancelled"
            && crate::delivery::has_active(c, scope, co, old.id).await?
        {
            return Err(invalid(
                "status",
                "Cancel draft deliveries first. An order with shipped or delivered quantities cannot be cancelled.",
            ));
        }
        p.reason = if to == "cancelled" || to == "rejected" {
            required(v, "reason", 500)?
        } else {
            text(v, "reason").to_owned()
        };
        if p.reason.chars().count() > 500 {
            return Err(invalid("reason", "Use up to 500 characters."));
        }
        p.target = to;
        return Ok(p);
    }
    if v["quotation_id"].as_i64().is_some() && v["quotation_number"].as_i64().is_some() {
        return Err(invalid(
            "quotation_id",
            "Choose a quotation by its ID or its number, not both.",
        ));
    }
    let mut from_quote = None;
    if kind == "order"
        && (v["quotation_id"].as_i64().is_some() || v["quotation_number"].as_i64().is_some())
    {
        if v.get("customer_code").is_some_and(|x| !x.is_null())
            || v.get("lines").is_some_and(|x| !x.is_null())
        {
            return Err(invalid(
                "quotation_id",
                "An order from a quotation uses that quotation's customer and lines. Remove the customer and lines, or clear the quotation.",
            ));
        }
        let id = if v["quotation_id"].as_i64().is_some() {
            integer(v, "quotation_id")?
        } else {
            let number = integer(v, "quotation_number")?;
            c.query_opt("SELECT id FROM commerce_sales_documents WHERE tenant_id=$1 AND company_id=$2 AND kind='quotation' AND number=$3",&[&scope.tenant(),&co,&number]).await?.ok_or_else(||invalid("quotation_number","That quotation is not available."))?.try_get(0).map_err(bilibi_database::Error::from)?
        };
        let quote = store::get(c, scope.tenant(), co, "quotation", id)
            .await?
            .ok_or(Error::NotFound)?;
        if quote.status != "accepted" {
            return Err(invalid("quotation_id", "Choose an accepted quotation."));
        }
        if c.query_opt("SELECT id FROM commerce_sales_documents WHERE tenant_id=$1 AND company_id=$2 AND quotation_id=$3",&[&scope.tenant(),&co,&id]).await?.is_some() {return Err(invalid("quotation_id","A sales order already exists for this quotation."));}
        for field in ["currency_code", "customer_kind"] {
            let given = text(v, field);
            if !given.is_empty() && given != text(&quote.facts, field) {
                return Err(invalid(
                    field,
                    "This differs from the quotation. Clear it to use the quotation's value.",
                ));
            }
        }
        from_quote = Some((id, quote));
    }
    let (customer_kind, customer_code) = match &from_quote {
        Some((_, q)) => (
            text(&q.facts, "customer_kind").to_owned(),
            text(&q.facts, "customer_code").to_owned(),
        ),
        None => (
            required(v, "customer_kind", 20)?,
            required(v, "customer_code", 255)?,
        ),
    };
    if !["company", "person"].contains(&customer_kind.as_str()) {
        return Err(invalid("customer_kind", "Choose Company or Person."));
    }
    let identity = parties
        .parties_by_codes(c, scope, std::slice::from_ref(&customer_code))
        .await?
        .into_iter()
        .find(|i| i.record_kind() == customer_kind)
        .ok_or_else(|| {
            invalid(
                "customer_code",
                "That Core company or person is not available.",
            )
        })?;
    records
        .check(c, scope, &customer_kind, &identity.record_id().to_string())
        .await?
        .map_err(|_| {
            invalid(
                "customer_code",
                "You cannot use that Core company or person.",
            )
        })?;
    if identity.status != "active" || (customer_kind == "company" && identity.record_id() == co) {
        return Err(invalid(
            "customer_code",
            "Choose an active customer other than the current company.",
        ));
    }
    if let Some((id, quote)) = from_quote {
        if text(v, "reference").chars().count() > 200 {
            return Err(invalid("reference", "Use up to 200 characters."));
        }
        currency(
            c,
            scope,
            text(&quote.facts, "currency_code"),
            currencies,
            tx,
        )
        .await?;
        p.facts = quote.facts;
        p.facts["reference"] = json!(text(v, "reference"));
        p.facts["quotation_id"] = json!(id);
        p.quotation = Some(id);
        return Ok(p);
    }
    let currency = currency(c, scope, text(v, "currency_code"), currencies, tx).await?;
    let places = currency.minor_units;
    let price_on = if text(v, "price_on").is_empty() {
        today()
    } else {
        date(v, "price_on")?
    };
    let on = price_on
        .parse::<bilibi_product::Date>()
        .map_err(|_| invalid("price_on", "Choose a pricing date."))?;
    let input = v["lines"]
        .as_array()
        .filter(|a| !a.is_empty() && a.len() <= 100)
        .ok_or_else(|| invalid("lines", "Add between 1 and 100 lines."))?;
    let mut lines = Vec::new();
    let mut subtotal = Decimal::ZERO;
    let mut tax_total = Decimal::ZERO;
    for (i, line) in input.iter().enumerate() {
        let mut source = line.clone();
        let mut product_code = Value::Null;
        let product_id = match line.get("product_id") {
            None | Some(Value::Null) => None,
            Some(_) => Some(integer(line, "product_id")?),
        };
        if let Some(id) = product_id {
            records
                .check(c, scope, "product", &id.to_string())
                .await?
                .map_err(|e| invalid("lines", e.message()))?;
            let sql = c.transaction().ok_or(bilibi_database::Error::Internal)?;
            let product = products.get(sql, scope, id).await?;
            let price = products.price(sql, scope, id, co, &currency.code, on).await?
                .ok_or_else(|| invalid("lines", "This product has no price for the selling company, currency and pricing date. Add a dated selling price in Products."))?;
            source["description"] = json!(product.name);
            source["unit"] = json!(product.base_unit);
            source["unit_price"] = json!(price.amount.to_string());
            product_code = json!(product.code);
        }
        let quantity = decimal(&source, "quantity", true)?;
        let price = money(decimal(&source, "unit_price", false)?, places);
        let rate = decimal(&source, "tax_rate", false)?;
        if rate > Decimal::from(100) {
            return Err(invalid(
                "tax_rate",
                "Tax rate must be from 0 to 100 percent.",
            ));
        }
        let net = money(
            quantity
                .checked_mul(price)
                .ok_or_else(|| invalid("lines", "The line amount is too large."))?,
            places,
        );
        let tax = money(net * rate / Decimal::from(100), places);
        subtotal += net;
        tax_total += tax;
        lines.push(json!({"line_number":i+1,"product_id":product_id,"product_code":product_code,"description":required(&source,"description",500)?,"unit":required(&source,"unit",20)?,"quantity":quantity.to_string(),"unit_price":price.to_string(),"tax_code":required(&source,"tax_code",50)?,"tax_rate":rate.to_string(),"subtotal":net.to_string(),"tax_total":tax.to_string(),"total":(net+tax).to_string()}));
    }
    let valid_until = if kind == "quotation" {
        let d = date(v, "valid_until")?;
        if d < today() {
            return Err(invalid("valid_until", "Choose today or a later date."));
        }
        Some(d)
    } else {
        None
    };
    p.facts = json!({"customer_kind":customer_kind,"customer_id":identity.record_id(),"customer_code":customer_code,"currency_code":currency.code,"price_on":price_on,"valid_until":valid_until,"reference":text(v,"reference"),"lines":lines,"subtotal":money(subtotal,places).to_string(),"tax_total":money(tax_total,places).to_string(),"total":money(subtotal+tax_total,places).to_string()});
    if text(v, "reference").chars().count() > 200 {
        return Err(invalid("reference", "Use up to 200 characters."));
    }
    Ok(p)
}
