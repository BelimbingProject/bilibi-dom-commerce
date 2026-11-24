use crate::{prepare::text, Record};
use bilibi_database::{Error, Result};
use bilibi_templates::{CompiledPrint, PrintFacts, PrintValue};
use jiff::civil::Date;
use serde_json::Value;

pub(super) fn render(
    compiled: &CompiledPrint<'_>,
    record: &Record,
    confirmed_on: Option<Date>,
    malay: bool,
    accent: bilibi_ui::Accent,
) -> Result<bilibi_artifacts::InlineDocument> {
    let facts = &record.facts;
    let state = match record.status.as_str() {
        "draft" => ["Draft", "Draf"],
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
                ("price", money(currency, line, "list_unit_price")?),
                ("discount", money(currency, line, "unit_discount")?),
                ("net", money(currency, line, "subtotal")?),
                ("tax_code", PrintValue::Text(&taxes[index])),
                ("tax", money(currency, line, "tax_total")?),
                ("total", money(currency, line, "total")?),
            ])
        })
        .collect::<Result<_>>()?;
    let rows: Vec<PrintFacts<'_>> = rows.iter().map(|row| row.as_slice()).collect();
    let issuer = text(facts, "issuer_name");
    let registration = text(facts, "issuer_registration");
    let registration_label = if malay {
        "No. pendaftaran"
    } else {
        "Registration no."
    };
    let issuer = if registration.is_empty() {
        issuer.to_owned()
    } else {
        format!("{issuer}\n{registration_label}: {registration}")
    };
    let customer = format!(
        "{}\n{}",
        text(facts, "customer_name"),
        text(facts, "customer_code")
    );
    let mut values = vec![
        ("number", PrintValue::Text(record.code.as_str())),
        ("state", PrintValue::Text(state)),
        ("issuer", PrintValue::Text(&issuer)),
        ("customer", PrintValue::Text(&customer)),
        ("lines", PrintValue::Rows(&rows)),
        ("subtotal", money(currency, facts, "subtotal")?),
        ("tax_total", money(currency, facts, "tax_total")?),
        ("total", money(currency, facts, "total")?),
    ];
    if record.kind == "quotation" {
        for key in ["price_on", "valid_until"] {
            let date = text(facts, key).parse().map_err(|_| Error::Invalid)?;
            values.push((key, PrintValue::Date(date)));
        }
    } else {
        values.push((
            "confirmed_on",
            PrintValue::Date(confirmed_on.ok_or(Error::NotFound)?),
        ));
        let reference = text(facts, "quotation_number");
        if !reference.is_empty() {
            values.push(("quotation", PrintValue::Text(reference)));
        }
    }
    for (field, fact) in [("address", "billing_address"), ("reference", "reference")] {
        let value = text(facts, fact);
        if !value.is_empty() {
            values.push((field, PrintValue::Text(value)));
        }
    }
    compiled
        .document(&record.status, &values, accent)
        .map_err(|_| Error::Invalid)
}

fn money<'a>(currency: &'a str, value: &Value, key: &str) -> Result<PrintValue<'a>> {
    Ok(PrintValue::Money {
        amount: text(value, key).parse().map_err(|_| Error::Invalid)?,
        currency,
    })
}
