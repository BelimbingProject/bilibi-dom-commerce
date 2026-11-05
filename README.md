# Commerce

Commerce manages quotations, sales orders and deliveries using shared Core products in Bilibi.

For example, create a Core product with a base unit and a dated selling price. Add it to a quotation for a Core company or person. Mark the quotation as sent, record acceptance, then create and confirm a sales order. Confirmation publishes the line snapshots that another Domain can use to prepare an invoice.

## Build and run

Commerce is a Domain, not a standalone Cargo workspace. Mount a real Git checkout into the Platform:

```sh
git clone https://github.com/BelimbingProject/bilibi.git bilibi
git clone https://github.com/BelimbingProject/bilibi-dom-commerce.git bilibi/apps/domains/commerce
python3 bilibi/scripts/discover-modules.py
```

Run the following commands from the Platform checkout:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
./scripts/build-assets.sh --check
./bilibi start
```

Read the Platform guides at `AGENTS.md`, `apps/AGENTS.md`, `apps/domains/AGENTS.md` and `apps/base/ui/AGENTS.md` before changing Commerce. The Platform owns shared UI components, styles, JavaScript, authentication, permissions, audit and event delivery. This repository has no root Cargo manifest or lockfile. CI mounts it into the Platform revision in `.github/bilibi-revision`.

## Selling

Each kind of record has one list and one detail page under **Commerce**:

| Page | What you can do |
| --- | --- |
| Quotations | Create a quotation, mark it sent, then accept, reject or expire it. |
| Sales orders | Create directly or copy an accepted quotation, then confirm or cancel. See delivered and remaining quantities on each line. |
| Delivery orders | Choose a confirmed sales order, save full or partial quantities, then ship and mark delivered. Print an A4 delivery order. |

A customer is a Core Company or Person. Enter its existing Core code and choose its type. Commerce proves that you can use that record and stores its kind and ID, without creating a separate customer identity.

Choose a product by name using the shared picker, then enter quantity, tax code and tax rate. Commerce copies the description, base unit and Core selling price when you save. The price is for the selected work company, currency and pricing date (today by default). A missing price stays missing: add a dated selling price on the Core Products page before saving. You can also leave the product unchosen and type description, quantity, unit, price and tax. A zero tax rate must be entered explicitly; Commerce supplies no company-specific tax codes, units, currency or prices.

Documents keep their original lines and totals. Product edits do not rewrite them. To correct a quotation before sending, create a replacement; documents have no draft editor in this first slice. Each accepted quotation can create at most one sales order. Copying it retains the quoted prices and tax, even if the product or selling price has changed, but the customer must still be active and usable; you cannot also enter a customer or lines, and a currency or customer type you enter must match the quotation. A reference (such as the customer's PO number) is kept on the order.

“Sent” records that you sent the quotation yourself; Commerce does not send email. “Accept” and “Reject” record the customer's decision entered by a member of staff. A quotation may be accepted or rejected through its valid-until date. It can be expired after that date, using UTC calendar dates. Expiry is a staff action; there is no scheduled expiry job.

Confirmation freezes the order for the event handoff. Cancellation remains available and publishes a separate status fact so a future consumer can reconcile a cancelled order. Cancelling an order does not reverse an invoice.

The Finance consumer, stock, curated catalogs and discount tiers are follow-up work. The repository does not mount itself into a published Bilibi application.

## Where to go next

- [Sales contracts and versioned events](sales/README.md)
- [Bilibi Platform](https://github.com/BelimbingProject/bilibi)

## File length

A pull request may not add a Rust file over 800 lines, and may not make a file that is already over 800 lines longer. Split it or keep it from growing.

- Run `python3 scripts/file-length-ratchet.py` before you push; it compares your branch with `origin/main`.
- Files under `fixtures/`, `examples/`, `migrations/` and `generated/` are skipped.
- CI runs it as `.github/workflows/file-length.yml`. The same script is copied unchanged from the Bilibi Platform into every Bilibi repository.
