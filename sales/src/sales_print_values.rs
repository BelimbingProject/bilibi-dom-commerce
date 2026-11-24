use crate::{Record, prepare::text};
use bilibi_database::{Error, Result};
use bilibi_templates::{CompiledPrint, PrintFacts, PrintValue};
use jiff::civil::Date;
use serde_json::Value;

pub(super) fn render(
    compiled: &CompiledPrint<'_>,
    record: &Record,
    confirmed_on: Option<Date>,
    quotation_number: Option<&str>,
    malay: bool,
    accent: bilibi_ui::Accent,
) -> Result<bilibi_artifacts::InlineDocument> {
    let facts = &record.facts;
    let state = match record.status.as_str() {
        "sent" => ["Sent", "Dihantar"],
        "accepted" => ["Accepted", "Diterima"],
        "rejected" => ["Rejected", "Ditolak"],
        "expired" => ["Expired", "Tamat tempoh"],
        "confirmed" => ["Confirmed", "Disahkan"],
        "cancelled" => ["Cancelled", "Dibatalkan"],
        _ => return Err(Error::Invalid),
    }[usize::from(malay)];
    let currency = text(facts, "currency_code");
    let raw_rows = facts["lines"].as_array().ok_or(Error::Invalid)?;
    let descriptions: Vec<_> = raw_rows
        .iter()
        .map(|line| {
            let name = text(line, "description");
            let code = text(line, "product_code");
            if code.is_empty() {
                name.to_owned()
            } else {
                format!("{code}\n{name}")
            }
        })
        .collect();
    let quantities: Vec<_> = raw_rows
        .iter()
        .map(|line| format!("{} {}", text(line, "quantity"), text(line, "unit")))
        .collect();
    let taxes: Vec<_> = raw_rows
        .iter()
        .map(|line| format!("{} {}%", text(line, "tax_code"), text(line, "tax_rate")))
        .collect();
    let rows: Vec<_> = raw_rows
        .iter()
        .enumerate()
        .map(|(index, line)| {
            Ok([
                ("item", PrintValue::Text(&descriptions[index])),
                ("quantity", PrintValue::Text(&quantities[index])),
                ("price", list_price(currency, line)?),
                ("discount", unit_discount(currency, line)?),
                ("net", money(currency, line, "subtotal")?),
                ("tax_code", PrintValue::Text(&taxes[index])),
                ("tax", money(currency, line, "tax_total")?),
                ("total", money(currency, line, "total")?),
            ])
        })
        .collect::<Result<_>>()?;
    let rows: Vec<PrintFacts<'_>> = rows.iter().map(|row| row.as_slice()).collect();
    let customer = [text(facts, "customer_name"), text(facts, "customer_code")]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let mut values = vec![
        ("number", PrintValue::Text(record.code.as_str())),
        ("state", PrintValue::Text(state)),
        ("customer", PrintValue::Text(&customer)),
        ("lines", PrintValue::Rows(&rows)),
        ("subtotal", money(currency, facts, "subtotal")?),
        ("tax_total", money(currency, facts, "tax_total")?),
        ("total", money(currency, facts, "total")?),
    ];
    if record.kind == "quotation" {
        quotation_dates(facts, &mut values)?;
    } else {
        values.push((
            "confirmed_on",
            PrintValue::Date(confirmed_on.ok_or(Error::NotFound)?),
        ));
        if let Some(quotation_number) = quotation_number {
            values.push(("quotation", PrintValue::Text(quotation_number)));
        }
    }
    for (field, fact) in [
        ("issuer", "issuer_name"),
        ("address", "billing_address"),
        ("reference", "reference"),
    ] {
        let value = text(facts, fact);
        if !value.is_empty() {
            values.push((field, PrintValue::Text(value)));
        }
    }
    compiled
        .document(&record.status, &values, accent)
        .map_err(|_| Error::Invalid)
}

fn quotation_dates(facts: &Value, values: &mut Vec<(&str, PrintValue<'_>)>) -> Result<()> {
    if let Some(price_on) = facts.get("price_on") {
        let date = price_on
            .as_str()
            .ok_or(Error::Invalid)?
            .parse()
            .map_err(|_| Error::Invalid)?;
        values.push(("price_on", PrintValue::Date(date)));
    }
    let expiry = text(facts, "valid_until")
        .parse()
        .map_err(|_| Error::Invalid)?;
    values.push(("valid_until", PrintValue::Date(expiry)));
    Ok(())
}

fn money<'a>(currency: &'a str, value: &Value, key: &str) -> Result<PrintValue<'a>> {
    Ok(PrintValue::Money {
        amount: text(value, key).parse().map_err(|_| Error::Invalid)?,
        currency,
    })
}

fn list_price<'a>(currency: &'a str, line: &Value) -> Result<PrintValue<'a>> {
    let key = if line.get("list_unit_price").is_some() {
        "list_unit_price"
    } else {
        "unit_price"
    };
    money(currency, line, key)
}

fn unit_discount<'a>(currency: &'a str, line: &Value) -> Result<PrintValue<'a>> {
    if line.get("unit_discount").is_some() {
        money(currency, line, "unit_discount")
    } else {
        Ok(PrintValue::Money {
            amount: rust_decimal::Decimal::ZERO,
            currency,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sales_layout::{ORDER, QUOTATION};
    use serde_json::json;

    #[test]
    fn pre_upgrade_sales_records_compile_to_printable_documents() {
        let mut record = Record {
            id: 1,
            revision: 2,
            kind: "quotation".into(),
            code: "42".into(),
            status: "sent".into(),
            facts: json!({
                "currency_code": "MYR",
                "customer_code": "CUSTOMER-1",
                "valid_until": "2026-01-31",
                "lines": [{
                    "description": "Carton",
                    "quantity": "2",
                    "unit": "EA",
                    "unit_price": "50.00",
                    "tax_code": "SV6",
                    "tax_rate": "6",
                    "subtotal": "100.00",
                    "tax_total": "6.00",
                    "total": "106.00"
                }],
                "subtotal": "100.00",
                "tax_total": "6.00",
                "total": "106.00"
            }),
        };
        let line = &record.facts["lines"][0];
        assert!(matches!(
            list_price("MYR", line).unwrap(),
            PrintValue::Money { amount, currency: "MYR" } if amount == rust_decimal::Decimal::from(50)
        ));
        assert!(matches!(
            unit_discount("MYR", line).unwrap(),
            PrintValue::Money { amount, currency: "MYR" } if amount == rust_decimal::Decimal::ZERO
        ));
        let mut dates = Vec::new();
        quotation_dates(&record.facts, &mut dates).unwrap();
        assert!(matches!(
            dates.as_slice(),
            [("valid_until", PrintValue::Date(date))] if *date == jiff::civil::date(2026, 1, 31)
        ));
        for default in QUOTATION.defaults {
            let quotation = QUOTATION.compile(default.language, default.layout).unwrap();
            assert!(
                render(
                    &quotation,
                    &record,
                    None,
                    None,
                    default.language == "ms",
                    Default::default(),
                )
                .is_ok()
            );
        }

        record.kind = "order".into();
        record.status = "confirmed".into();
        record.code = "12".into();
        let order = ORDER.compile("ms", ORDER.defaults[1].layout).unwrap();
        let confirmed_on = jiff::civil::date(2026, 1, 3);
        assert!(
            render(
                &order,
                &record,
                Some(confirmed_on),
                Some("42"),
                true,
                Default::default(),
            )
            .is_ok()
        );
    }
}
