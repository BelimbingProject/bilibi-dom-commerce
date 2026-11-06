//! A company dropped on Quotations or Sales orders, or carried there from a
//! Companies row's actions menu (`?drop=company:42`), opens that page's own
//! "New quotation" or "New sales order" review with the company chosen as the
//! customer, as if "+ Quotation" had been pressed and its code typed. The drop
//! writes nothing: the review commits the page's create contract, which checks
//! the customer again (`prepare.rs`).
use super::State;
use crate::{contributions::PAGES, MANAGE};
use bilibi_database::{Error, Result};
use bilibi_party_ids::{OrganisationId, PartyId};
use bilibi_tenancy::Scope;
use bilibi_ui::{Html, StatusKind};

pub(super) const QUOTATION_TARGET: &str = "commerce.sales.quotation.new";
pub(super) const ORDER_TARGET: &str = "commerce.sales.order.new";
const DENIED: &str = "You cannot create documents for this company.";

bilibi_records::drop_target!(
    "commerce/sales",
    bilibi_records::DropTarget {
        id: QUOTATION_TARGET,
        service: PAGES,
        contract: "commerce.sales.quotation.create",
        capability: MANAGE,
        path: "/commerce/quotations",
        label: "New quotation",
        accepts: &["company"],
        max: 1,
    }
);
bilibi_records::drop_target!(
    "commerce/sales",
    bilibi_records::DropTarget {
        id: ORDER_TARGET,
        service: PAGES,
        contract: "commerce.sales.order.create",
        capability: MANAGE,
        path: "/commerce/sales-orders",
        label: "New sales order",
        accepts: &["company"],
        max: 1,
    }
);

/// The target of a document list; a record page and Delivery orders take none.
fn target(kind: &str) -> Option<&'static str> {
    match kind {
        "quotation" => Some(QUOTATION_TARGET),
        "order" => Some(ORDER_TARGET),
        _ => None,
    }
}

impl State {
    /// The drop target this page is, while its "+ New" button shows.
    fn drop_target(&self) -> Option<&'static bilibi_records::DropTarget> {
        target(self.kind)
            .filter(|_| self.id.is_none() && self.manage)
            .and_then(|id| self.records.drop_target(id))
    }

    /// The list wrapped as a drop zone when it accepts a company.
    pub(super) fn drop_zone(&self, html: Html) -> Html {
        match self.drop_target() {
            Some(target) => target.zone().wrap("commerce-drop", html),
            None => html,
        }
    }

    /// Opens the new-document review on a dropped company. The package is a
    /// request only: `Records::receive` re-reads it under this person's scope,
    /// and the create contract decides again when the review is saved.
    pub(super) async fn receive_drop(&mut self, scope: &Scope, text: &str) -> Result<()> {
        let Some(target) = target(self.kind).filter(|_| self.id.is_none()) else {
            return Err(Error::Invalid);
        };
        if !self.manage {
            self.flash = Some((StatusKind::Error, DENIED.into()));
            return Ok(());
        }
        let dropped = match self.records.receive(scope, target, text).await? {
            Ok(dropped) => dropped,
            Err(refusal) => {
                self.flash = Some((StatusKind::Error, refusal.message()));
                return Ok(());
            }
        };
        // The target takes one company, so there is exactly one.
        let company = dropped[0]
            .id
            .parse::<i64>()
            .ok()
            .and_then(|id| OrganisationId::new(id).ok())
            .ok_or(Error::Invalid)?;
        let c = self.sales.db.get().await?;
        let identity = self
            .parties
            .party_identity(&**c, scope, PartyId::Organisation(company))
            .await?;
        drop(c);
        self.open_new();
        self.form.insert("customer_kind".into(), "company".into());
        self.form.insert("customer_code".into(), identity.code);
        Ok(())
    }
}
