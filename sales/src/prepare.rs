//! All business validation and exact arithmetic live here, shared by preview
//! and commit. Catalog references resolve under the company lock.
use crate::{store, Record};
use bilibi_agent_api::{Error, OperationClient, Result};
use bilibi_records::Records;
use bilibi_tenancy::{PartyProofs, Scope};
use rust_decimal::{Decimal, RoundingStrategy};
use serde_json::{json, Value};
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
fn required(v: &Value, k: &str, max: usize) -> Result<String> {
    let s = text(v, k);
    if s.is_empty() || s.chars().count() > max || s.contains('\0') {
        return Err(invalid(k, "Enter a value within the field limit."));
    }
    Ok(s.into())
}
fn decimal(v: &Value, k: &str, positive: bool) -> Result<Decimal> {
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
fn money(d: Decimal) -> Decimal {
    let mut d = d.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
    d.rescale(2);
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

pub(crate) async fn prepare(
    c: &dyn OperationClient,
    scope: &Scope,
    v: &Value,
    action: &str,
    records: &Records,
    parties: &PartyProofs,
) -> Result<Prepared> {
    let co = integer(v, "company_id")?;
    let kind = if action.starts_with("item") {
        "item"
    } else if action.starts_with("quotation") {
        "quotation"
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
    if action.ends_with("transition") || action == "item.update" {
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
    if kind == "item" {
        if p.old.is_none() {
            p.code = required(v, "code", 50)?;
        }
        if p.old.is_none() && c.query_opt("SELECT id FROM commerce_sales_items WHERE tenant_id=$1 AND company_id=$2 AND code=$3",&[&scope.tenant(),&co,&p.code]).await?.is_some() {return Err(invalid("code","This item code is already in use."));}
        let rate = decimal(v, "tax_rate", false)?;
        if rate > Decimal::from(100) {
            return Err(invalid(
                "tax_rate",
                "Tax rate must be from 0 to 100 percent.",
            ));
        }
        p.facts = json!({"name":required(v,"name",200)?,"unit":required(v,"unit",20)?,"default_price":decimal(v,"default_price",false)?.to_string(),"tax_code":required(v,"tax_code",50)?,"tax_rate":rate.to_string(),"active":v["active"].as_bool().unwrap_or(true)});
        p.target = "active".into();
        return Ok(p);
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
    if kind == "order" && v["quotation_id"].as_i64().is_some() {
        let id = integer(v, "quotation_id")?;
        let quote = store::get(c, scope.tenant(), co, "quotation", id)
            .await?
            .ok_or(Error::NotFound)?;
        if quote.status != "accepted" {
            return Err(invalid("quotation_id", "Choose an accepted quotation."));
        }
        if c.query_opt("SELECT id FROM commerce_sales_documents WHERE tenant_id=$1 AND company_id=$2 AND quotation_id=$3",&[&scope.tenant(),&co,&id]).await?.is_some() {return Err(invalid("quotation_id","A sales order already exists for this quotation."));}
        p.facts = quote.facts;
        p.facts["quotation_id"] = json!(id);
        p.quotation = Some(id);
        return Ok(p);
    }
    let customer_kind = required(v, "customer_kind", 20)?;
    if !["company", "person"].contains(&customer_kind.as_str()) {
        return Err(invalid("customer_kind", "Choose Company or Person."));
    }
    let customer_code = required(v, "customer_code", 255)?;
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
    let currency = required(v, "currency_code", 3)?;
    if currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err(invalid(
            "currency_code",
            "Enter a three-letter currency code.",
        ));
    }
    let input = v["lines"]
        .as_array()
        .filter(|a| !a.is_empty() && a.len() <= 100)
        .ok_or_else(|| invalid("lines", "Add between 1 and 100 lines."))?;
    let mut lines = Vec::new();
    let mut subtotal = Decimal::ZERO;
    let mut tax_total = Decimal::ZERO;
    for (i, line) in input.iter().enumerate() {
        let mut source = line.clone();
        let mut item_code = Value::Null;
        if let Some(id) = line["item_id"].as_i64() {
            let item = store::get(c, scope.tenant(), co, "item", id)
                .await?
                .ok_or_else(|| invalid("lines", "A catalog item is no longer available."))?;
            if item.facts["active"] == false {
                return Err(invalid("lines", "Choose active catalog items."));
            }
            source = json!({"description":item.facts["name"],"unit":item.facts["unit"],"unit_price":item.facts["default_price"],"tax_code":item.facts["tax_code"],"tax_rate":item.facts["tax_rate"],"quantity":line["quantity"]});
            item_code = json!(item.code);
        }
        let quantity = decimal(&source, "quantity", true)?;
        let price = decimal(&source, "unit_price", false)?;
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
        );
        let tax = money(net * rate / Decimal::from(100));
        subtotal += net;
        tax_total += tax;
        lines.push(json!({"line_number":i+1,"item_id":line["item_id"],"item_code":item_code,"description":required(&source,"description",500)?,"unit":required(&source,"unit",20)?,"quantity":quantity.to_string(),"unit_price":price.to_string(),"tax_code":required(&source,"tax_code",50)?,"tax_rate":rate.to_string(),"subtotal":net.to_string(),"tax_total":tax.to_string(),"total":(net+tax).to_string()}));
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
    p.facts = json!({"customer_kind":customer_kind,"customer_id":identity.record_id(),"customer_code":customer_code,"currency_code":currency,"valid_until":valid_until,"reference":text(v,"reference"),"lines":lines,"subtotal":money(subtotal).to_string(),"tax_total":money(tax_total).to_string(),"total":money(subtotal+tax_total).to_string()});
    if text(v, "reference").chars().count() > 200 {
        return Err(invalid("reference", "Use up to 200 characters."));
    }
    Ok(p)
}
