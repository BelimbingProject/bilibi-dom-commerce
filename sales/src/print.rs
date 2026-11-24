//! Sales document downloads use the same private render path.
use crate::{
    delivery_layout::CONTRACT, prepare::text, sales_layout, sales_print_values, Record, Sales, VIEW,
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

pub(crate) struct SalesPrint {
    sales: Arc<Sales>,
    artifacts: Arc<Artifacts>,
    templates: Arc<Templates>,
    branding: Arc<Branding>,
    settings: Arc<Settings>,
}

impl SalesPrint {
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

fn record_kind(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "delivery-order" => "delivery",
        "quotation" => "quotation",
        "order-confirmation" => "order",
        _ => return None,
    })
}
impl Owner for SalesPrint {
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
            let reference = reference.ok_or(Error::NotFound)?;
            let kind = record_kind(&reference.kind).ok_or(Error::NotFound)?;
            let id = reference
                .subject
                .parse::<i64>()
                .ok()
                .filter(|id| *id > 0)
                .ok_or(Error::NotFound)?;
            c.query_opt("SELECT id FROM commerce_sales_documents WHERE tenant_id=$1 AND company_id=$2 AND kind=$3 AND id=$4", &[&s.tenant(),&company.get(),&kind,&id]).await?.ok_or(Error::NotFound)?;
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
        ("issuer", PrintValue::Text(text(facts, "issuer_name"))),
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

impl Download<Scope, Error> for SalesPrint {
    fn serve<'a>(&'a self, r: DownloadRequest<'a, Scope>) -> DownloadFuture<'a, Error> {
        Box::pin(async move {
            let (kind, reference_kind, contract, state_date) = match r.operation {
                "delivery_pdf" => ("delivery", "delivery-order", &CONTRACT, Some("shipped")),
                "quotation_pdf" => ("quotation", "quotation", &sales_layout::QUOTATION, None),
                "order_confirmation_pdf" => (
                    "order",
                    "order-confirmation",
                    &sales_layout::ORDER,
                    Some("confirmed"),
                ),
                _ => return Err(Error::NotFound),
            };
            let company = r.scope.current_company().ok_or(Error::Forbidden)?;
            let id = r
                .path
                .get("id")
                .and_then(|id| id.parse::<i64>().ok())
                .filter(|id| *id > 0)
                .ok_or(Error::NotFound)?;
            let record = self.sales.get(r.scope, company.get(), kind, id).await?;
            let event_date = if let Some(state) = state_date.filter(|_| record.status != "draft") {
                self.sales
                    .history(r.scope, company.get(), id)
                    .await?
                    .iter()
                    .find(|event| event["to"] == state)
                    .and_then(|event| event["at"].as_str()?.parse::<jiff::Timestamp>().ok())
                    .map(|when| when.to_zoned(jiff::tz::TimeZone::UTC).date())
            } else {
                None
            };
            if kind == "order" && event_date.is_none() {
                return Err(Error::NotFound);
            }
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
                    contract.key,
                    Some(company),
                    language.value.as_str().unwrap_or("en"),
                    None,
                )
                .await?;
            let compiled = contract
                .compile(&selected.language, &selected.layout)
                .map_err(|_| Error::Invalid)?;
            let accent = self.branding.accent(&**client).await?;
            let malay = selected.language.split('-').next() == Some("ms");
            let inline = if kind == "delivery" {
                document(&compiled, &record, event_date, malay, accent)?
            } else {
                sales_print_values::render(&compiled, &record, event_date, malay, accent)?
            };
            let version = match selected.origin {
                Origin::Packaged { version } => format!("packaged-{version}"),
                Origin::Company { revision, .. } | Origin::Tenant { revision } => {
                    format!("revision-{revision}")
                }
            };
            let options = PdfOptions::default()
                .template_version(format!("{}-{version}", selected.language))
                .data_version(format!("{}-revision-{}", contract.version, record.revision));
            let documents = self.artifacts.for_owner(r.scope, company, self);
            let metadata = documents
                .render_inline_document(
                    &mut client,
                    Reference {
                        subject: id.to_string(),
                        kind: reference_kind.into(),
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
                &format!("{reference_kind}-{id}.pdf"),
            ))
        })
    }
}
