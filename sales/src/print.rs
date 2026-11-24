//! The existing delivery download backed by the shared print compiler.
use crate::{
    delivery_layout::{CONTRACT, KEY},
    prepare::text,
    Record, Sales, VIEW,
};
use bilibi_artifacts::{Artifacts, Operation, Owner, PdfOptions, Reference};
use bilibi_branding::Branding;
use bilibi_database::{Error, Result};
use bilibi_party_ids::OrganisationId;
use bilibi_settings::{Settings, Target};
use bilibi_templates::{CompiledPrint, Origin, PrintFacts, PrintValue, Templates};
use bilibi_tenancy::{ProofClient, ProofFuture, Scope};
use bilibi_ui::download::{Download, DownloadFuture, DownloadRequest, FileBody};
use jiff::civil::Date;
use std::sync::Arc;

pub(crate) const SERVICE: &str = "commerce/sales.delivery-print";

pub(crate) struct DeliveryPrint {
    sales: Arc<Sales>,
    artifacts: Arc<Artifacts>,
    templates: Arc<Templates>,
    branding: Arc<Branding>,
    settings: Arc<Settings>,
}

impl DeliveryPrint {
    pub fn new(
        sales: Arc<Sales>,
        artifacts: Arc<Artifacts>,
        templates: Arc<Templates>,
        branding: Arc<Branding>,
        settings: Arc<Settings>,
    ) -> Self {
        Self {
            sales,
            artifacts,
            templates,
            branding,
            settings,
        }
    }
}

fn id(reference: Option<&Reference>) -> Result<i64> {
    reference
        .filter(|r| r.kind == "delivery-order")
        .and_then(|r| r.subject.parse().ok())
        .filter(|id| *id > 0)
        .ok_or(Error::NotFound)
}
impl Owner for DeliveryPrint {
    fn owner_id(&self) -> &'static str {
        "commerce/sales"
    }
    fn authorize<'a>(
        &'a self,
        c: &'a dyn ProofClient,
        s: &'a Scope,
        company: OrganisationId,
        operation: Operation,
        reference: Option<&'a Reference>,
    ) -> ProofFuture<'a, ()> {
        Box::pin(async move {
            if !matches!(operation, Operation::Create | Operation::Read) {
                return Err(Error::Forbidden);
            }
            bilibi_company::authorize_work_target(c, s, company.get(), VIEW).await?;
            let id = id(reference)?;
            c.query_opt("SELECT id FROM commerce_sales_documents WHERE tenant_id=$1 AND company_id=$2 AND kind='delivery' AND id=$3", &[&s.tenant(),&company.get(),&id]).await?.ok_or(Error::NotFound)?;
            Ok(())
        })
    }
}

fn document(
    compiled: &CompiledPrint<'_>,
    record: &Record,
    ship_date: Option<Date>,
    malay: bool,
    accent: bilibi_ui::Accent,
) -> Result<bilibi_artifacts::InlineDocument> {
    let facts = &record.facts;
    let status = match (malay, record.status.as_str()) {
        (false, "draft") => "Draft",
        (false, "shipped") => "Shipped",
        (false, "delivered") => "Delivered",
        (false, "cancelled") => "Cancelled",
        (true, "draft") => "Draf",
        (true, "shipped") => "Dihantar",
        (true, "delivered") => "Telah diterima",
        (true, "cancelled") => "Dibatalkan",
        _ => return Err(Error::Invalid),
    };
    let issuer_name = text(facts, "issuer_name");
    let issuer_reg = text(facts, "issuer_registration");
    let issuer = if issuer_reg.is_empty() {
        issuer_name.to_owned()
    } else {
        let label = if malay {
            "No. pendaftaran"
        } else {
            "Registration no."
        };
        format!("{issuer_name}\n{label}: {issuer_reg}")
    };
    let address = match text(facts, "ship_to") {
        "" if malay => "Alamat belum direkodkan",
        "" => "Address has not been recorded",
        address => address,
    };
    let mut rows = Vec::new();
    for line in facts["lines"].as_array().ok_or(Error::Invalid)? {
        let mut fields = vec![
            (
                "order_line",
                PrintValue::Integer(line["order_line_number"].as_i64().ok_or(Error::Invalid)?),
            ),
            ("item", PrintValue::Text(text(line, "description"))),
            ("unit", PrintValue::Text(text(line, "unit"))),
            (
                "ordered",
                PrintValue::Decimal(
                    text(line, "ordered_quantity")
                        .parse()
                        .map_err(|_| Error::Invalid)?,
                ),
            ),
            (
                "quantity",
                PrintValue::Decimal(text(line, "quantity").parse().map_err(|_| Error::Invalid)?),
            ),
        ];
        let product = text(line, "product_code");
        if !product.is_empty() {
            fields.push(("product", PrintValue::Text(product)));
        }
        rows.push(fields);
    }
    let rows: Vec<PrintFacts<'_>> = rows.iter().map(|fields| fields.as_slice()).collect();
    let mut values = vec![
        ("number", PrintValue::Text(&record.code)),
        ("issuer", PrintValue::Text(&issuer)),
        ("recipient", PrintValue::Text(text(facts, "customer_name"))),
        ("ship_to", PrintValue::Text(address)),
        ("order", PrintValue::Text(text(facts, "order_number"))),
        ("status", PrintValue::Text(status)),
        ("lines", PrintValue::Rows(&rows)),
    ];
    if let Some(date) = ship_date {
        values.push(("ship_date", PrintValue::Date(date)));
    }
    let reference = text(facts, "reference");
    if !reference.is_empty() {
        values.push(("reference", PrintValue::Text(reference)));
    }
    compiled
        .document(&record.status, &values, accent)
        .map_err(|_| Error::Invalid)
}

impl Download<Scope, Error> for DeliveryPrint {
    fn serve<'a>(&'a self, r: DownloadRequest<'a, Scope>) -> DownloadFuture<'a, Error> {
        Box::pin(async move {
            if r.operation != "delivery_pdf" {
                return Err(Error::NotFound);
            }
            let company = r.scope.current_company().ok_or(Error::Forbidden)?;
            let id = r
                .path
                .get("id")
                .and_then(|id| id.parse::<i64>().ok())
                .filter(|id| *id > 0)
                .ok_or(Error::NotFound)?;
            let record = self
                .sales
                .get(r.scope, company.get(), "delivery", id)
                .await?;
            let ship_date = if record.status == "draft" {
                None
            } else {
                self.sales
                    .history(r.scope, company.get(), id)
                    .await?
                    .iter()
                    .find(|event| event["to"] == "shipped")
                    .and_then(|event| event["at"].as_str()?.parse::<jiff::Timestamp>().ok())
                    .map(|when| when.to_zoned(jiff::tz::TimeZone::UTC).date())
            };
            let mut client = self.sales.db.get().await?;
            let language = self
                .settings
                .get(
                    &**client,
                    r.scope,
                    Target::Company(company),
                    bilibi_templates::LANGUAGE,
                )
                .await?;
            let selected = self
                .templates
                .resolve_print(
                    &**client,
                    r.scope,
                    KEY,
                    Some(company),
                    language.value.as_str().unwrap_or("en"),
                    None,
                )
                .await?;
            let compiled = CONTRACT
                .compile(&selected.language, &selected.layout)
                .map_err(|_| Error::Invalid)?;
            let accent = self.branding.accent(&**client).await?;
            let inline = document(
                &compiled,
                &record,
                ship_date,
                selected.language.split('-').next() == Some("ms"),
                accent,
            )?;
            let version = match selected.origin {
                Origin::Packaged { version } => format!("packaged-{version}"),
                Origin::Company { revision, .. } | Origin::Tenant { revision } => {
                    format!("revision-{revision}")
                }
            };
            let options = PdfOptions::default()
                .template_version(format!("{}-{version}", selected.language))
                .data_version(format!("{}-revision-{}", CONTRACT.version, record.revision));
            let documents = self.artifacts.for_owner(r.scope, company, self);
            let metadata = documents
                .render_inline_document(
                    &mut client,
                    Reference {
                        subject: id.to_string(),
                        kind: "delivery-order".into(),
                    },
                    inline,
                    &options,
                )
                .await
                .map_err(|_| Error::Internal)?;
            let document = documents
                .read(&mut client, &metadata.document.id)
                .await
                .map_err(|_| Error::Internal)?;
            Ok(FileBody::inline(
                document.bytes,
                "application/pdf",
                &format!("delivery-order-{id}.pdf"),
            ))
        })
    }
}
