//! References use the same company reach and view capability as Sales pages.
use crate::VIEW;
use bilibi_database::Error;
use bilibi_grid::{Field, FieldType, GridContribution, Source, TableDecl};
use bilibi_records::{Access, RecordKind, RecordOwner};
use bilibi_tenancy::{ProofClient, ProofFuture, Scope};

struct Owner(&'static str);
static ITEM: Owner = Owner("item");
static QUOTATION: Owner = Owner("quotation");
static ORDER: Owner = Owner("order");
impl RecordOwner for Owner {
    fn authorize<'a>(
        &'a self,
        client: &'a dyn ProofClient,
        scope: &'a Scope,
        id: &'a str,
    ) -> ProofFuture<'a, Access> {
        Box::pin(async move {
            let Some(id) = id.parse::<i64>().ok().filter(|id| *id > 0) else {
                return Ok(Access::NotFound);
            };
            let row = if self.0 == "item" {
                client
                    .query_opt(
                        "SELECT company_id FROM commerce_sales_items WHERE tenant_id=$1 AND id=$2",
                        &[&scope.tenant(), &id],
                    )
                    .await?
            } else {
                client.query_opt("SELECT company_id FROM commerce_sales_documents WHERE tenant_id=$1 AND id=$2 AND kind=$3", &[&scope.tenant(), &id, &self.0]).await?
            };
            let Some(row) = row else {
                return Ok(Access::NotFound);
            };
            match bilibi_company::authorize_work_target(
                client,
                scope,
                row.try_get("company_id")?,
                VIEW,
            )
            .await
            {
                Ok(_) => Ok(Access::Readable),
                Err(Error::NotFound) => Ok(Access::NotFound),
                Err(Error::Forbidden) => Ok(Access::Forbidden),
                Err(error) => Err(error),
            }
        })
    }
}
bilibi_records::declare!(
    "commerce/sales",
    RecordKind::new("commerce.sales.item", "commerce-item", &ITEM).page("/commerce/items/{id}")
);
bilibi_records::declare!(
    "commerce/sales",
    RecordKind::new("commerce.sales.quotation", "commerce-quotation", &QUOTATION)
        .page("/commerce/quotations/{id}")
);
bilibi_records::declare!(
    "commerce/sales",
    RecordKind::new("commerce.sales.order", "commerce-order", &ORDER)
        .page("/commerce/sales-orders/{id}")
);

// Grid's work-company source predicate matches other company-owned Domains.
const FIELDS: &[Field] = &[
    Field::new("id", "ID", FieldType::Integer),
    Field::new("code", "Number or code", FieldType::String),
];
static GRID: GridContribution = GridContribution {
    tables: &[
        TableDecl::new("commerce-item", "Products", VIEW, Source::tenant("SELECT id, code FROM commerce_sales_items WHERE tenant_id=$1 AND company_id IN (SELECT d.company_id FROM user_desks d JOIN users u ON u.tenant_id=d.tenant_id AND u.person_id=d.person_id AND u.kind='staff' AND u.deleted_at IS NULL WHERE d.tenant_id=$1 AND u.id=$2)"), "id", FIELDS).label_field("code").record_types(&["commerce.sales.item"]),
        TableDecl::new("commerce-quotation", "Quotations", VIEW, Source::tenant("SELECT id, number::text AS code FROM commerce_sales_documents WHERE tenant_id=$1 AND kind='quotation' AND company_id IN (SELECT d.company_id FROM user_desks d JOIN users u ON u.tenant_id=d.tenant_id AND u.person_id=d.person_id AND u.kind='staff' AND u.deleted_at IS NULL WHERE d.tenant_id=$1 AND u.id=$2)"), "id", FIELDS).label_field("code").record_types(&["commerce.sales.quotation"]),
        TableDecl::new("commerce-order", "Sales orders", VIEW, Source::tenant("SELECT id, number::text AS code FROM commerce_sales_documents WHERE tenant_id=$1 AND kind='order' AND company_id IN (SELECT d.company_id FROM user_desks d JOIN users u ON u.tenant_id=d.tenant_id AND u.person_id=d.person_id AND u.kind='staff' AND u.deleted_at IS NULL WHERE d.tenant_id=$1 AND u.id=$2)"), "id", FIELDS).label_field("code").record_types(&["commerce.sales.order"]),
    ],
    record_fields: &[],
    links: &[]
};
bilibi_grid::contribute!("commerce/sales", "commerce/sales.records", &GRID);
