# Sales contracts

The `commerce/sales` module owns quotations, sales orders, delivery orders and their status history.

## Writes

Browser actions and agents call the same registered safe-write operations. The browser uses `PageContracts`; an agent previews the operation, gets the person's approval, then commits the unchanged input. Every operation authorizes the explicit `company_id` under the sealed scope, locks that company's writable row, validates the facts, and writes through `MutationBoundary`.

| Operation | Required facts beyond `company_id` |
| --- | --- |
| `commerce.sales.quotation.create` | `customer_kind`, `customer_code`, `currency_code`, `valid_until`, `lines`; optional `reference` |
| `commerce.sales.quotation.transition` | `id`, current `revision`, `status`; `reason` for rejection |
| `commerce.sales.order.create` | Either `quotation_id` (or `quotation_number`, not both) or the same customer, currency and lines as a quotation; no expiry required. With a quotation, the customer and lines come from it and are checked again, so supplying `customer_code` or `lines` is refused. `reference` is kept on the order. `currency_code` and `customer_kind` are optional and refused only when they differ from the quotation |
| `commerce.sales.order.transition` | `id`, current `revision`, `status`; `reason` for cancellation |
| `commerce.sales.delivery.create` | `order_id` or `order_number` (not both); optional `reference` and `lines` with `order_line_number` and decimal `quantity`. Omit `lines` to deliver all available quantities |
| `commerce.sales.delivery.transition` | `id`, current `revision`, `status` (`shipped`, `delivered` or `cancelled`); `reason` for cancellation |

`customer_kind` is `company` or `person`. These are Core record kinds, with the same meaning as Finance receivables. A typed line has `description`, `quantity`, `unit`, `unit_price`, `tax_code` and `tax_rate`. A product line has a Core `product_id`, `quantity`, `tax_code` and `tax_rate`. Core supplies description, base unit and selling price; tax is a transaction fact. `price_on` selects the latest Core price on or before that date for the exact selling company and document currency, defaulting to today (UTC). A missing price refuses the save; zero is valid only when Core stores zero. New selections and saves call Records with kind `product`, so retired or inaccessible products are refused. The stored line includes all resolved facts, its 1-based `line_number`, and subtotal, tax and total.

Quantities, prices and rates are exact decimal strings with at most four decimal places and values at most 999,999,999. Quantities must be positive; prices and tax may be zero. Rates are percentages from 0 to 100. A document has 1–100 lines. Core Currency supplies the currencies (any current one; the used ones come first in the control) and their decimal places: JPY uses zero and KWD uses three. Line unit prices, subtotals, tax and totals round half away from zero to those places. Document totals add the rounded line amounts. Core prices must match the selling company and document currency; Commerce performs no conversion. A new document checks its currency with Core Currency on the write transaction, which marks a first use. Existing snapshots remain readable after ISO withdraws a currency.

Quotations move `draft → sent → accepted | rejected | expired`. Sales orders move `draft → confirmed → cancelled`, and drafts may be cancelled. A status change requires the current revision, so stale browser or agent actions are refused. Status history records creation and each transition with account, impersonator, reason and UTC instant. History and document facts are protected against changes and deletion by database triggers.

`commerce.sales.view` opens read pages. `commerce.sales.manage` permits writes; confirmation also requires `commerce.sales.confirm`. Tenant Owner receives all three through the vocabulary contribution. The company desk proof still applies to each call.

## Delivery orders

Open **Delivery orders**, choose **+ Delivery order**, enter a confirmed sales order number and load the lines. Each line shows ordered, delivered, remaining and available quantities. The form starts with all available quantities; reduce a quantity for a partial delivery, or leave it blank or enter zero to skip that line. Save the draft, review the change to shipped, then review the change to delivered when the customer receives it. A delivery cannot skip the shipped stage.

Draft and shipped deliveries reserve quantities. Available means ordered minus every non-cancelled delivery; remaining means ordered minus delivered. Cancelling a draft or shipped delivery requires a reason and releases its reservation. Delivered and cancelled records are final. Draft quantities are immutable: cancel and replace a draft to correct it. A source order with any non-cancelled delivery cannot be cancelled. This prevents a shipment from losing its confirmed source.

Every write holds the same company lock used by sales orders. Preview and commit both recalculate availability; a concurrent draft, stale revision, duplicate line, unknown source line, nonpositive quantity or quantity above availability is refused before any document, history or event is written. The company, order and every delivery query are scoped to the current tenant and explicit company.

The sales order record shows delivered, remaining and available quantities per line. These are current read values, not changes to the original order snapshot. Delivery snapshots retain the source order ID, number and revision, customer kind, ID and code, and each original product ID/code, description, unit and ordered quantity. The delivery quantity and 1-based source line number identify the partial delivery precisely. Deliveries have no prices or accounting totals, and do not change stock or invoices.

**Print delivery order** opens a private A4 PDF through Base Artifacts HTML rendering and the registered download contribution. The document shows the issuing company, customer code, source order, reference, status, quantities and signature spaces. Every string is escaped by Askama. The print adapter authorizes company work access and record visibility before rendering and again before publication/read. Base Artifacts owns Chromium, storage, retention and audit; configure its private document folder, retention days and PDF Chromium executable in Settings. No browser process or PDF storage is owned by Commerce.

## Record references

Other modules can refer to `commerce.sales.quotation`, `commerce.sales.order` and `commerce.sales.delivery` through Base Records, using the record ID as text. Names and links come from the matching Grid declaration. Each owner checks the tenant, document kind, company desk access and `commerce.sales.view` before revealing a record. Core owns the `product` kind and its single Products list and page. Commerce has no product master, product operations or additional product panel; current delivery quantities belong to documents and need no product-level facts.

## Actions on a company

Commerce registers two Base Records drop targets in `src/web/drops.rs`, so every Companies row offers them in its actions menu to people who hold `commerce.sales.manage`:

| Action | Opens | Commits when saved |
| --- | --- | --- |
| New quotation | Quotations, with the New quotation form open | `commerce.sales.quotation.create` |
| New sales order | Sales orders, with the New sales order form open | `commerce.sales.order.create` |

Each takes one company (`?drop=company:<id>`, or a company dropped on the list) and fills Customer type and Customer code with it. It saves nothing: you still add lines and save, and the create contract checks the customer again. A company you cannot see is refused with the Base Records message. Your own work company is refused at save, as when you type its code. Delivery orders and record pages take no drop.

## Events for invoice consumers

Commerce declares three Base event types, each at **version 1**:

- `commerce/sales.order_confirmed`: an order moved from draft to confirmed.
- `commerce/sales.order_status_changed`: an order was confirmed or cancelled.
- `commerce/sales.delivery_delivered`: a shipped delivery was marked delivered.

Publication happens on the order's audited transaction. Refusal or publication failure rolls back the complete write. There is no Finance dependency, port, SQL reference or invoice write.

Each event carries one line per part to stay within the Base event payload limits. The subject is `order:<id>` and the idempotency key is `order:<id>:revision:<revision>:<status|confirmed>:line:<line_number>`. The Base event envelope supplies tenant, event type version, actor and event time. The payload contains:

```json
{
  "company_id": 1,
  "order_id": 42,
  "order_number": "7",
  "revision": 2,
  "order_status": "confirmed",
  "quotation_id": null,
  "customer_kind": "company",
  "customer_id": 15,
  "customer_code": "C001",
  "currency_code": "MYR",
  "subtotal": "10.00",
  "tax_total": "0.00",
  "total": "10.00",
  "part_index": 1,
  "part_count": 1,
  "lines": [{
    "line_number": 1,
    "product_id": null,
    "product_code": null,
    "description": "Service",
    "quantity": "1",
    "unit": "hour",
    "unit_price": "10.00",
    "tax_code": "ZERO",
    "tax_rate": "0",
    "subtotal": "10.00",
    "tax_total": "0.00",
    "total": "10.00"
  }]
}
```

The example uses arbitrary company data, not built-in defaults. Consumers keep their own types and copy the parts they need. Deduplicate by event identity, collect every part for an order revision, and reconcile later cancellation from the status event. Do not create an invoice once per part. The Finance consumer is outside this slice.

The delivered event is recorded on the same audited transaction as the delivered status and history. It uses subject `delivery:<id>` and key `delivery:<id>:revision:<revision>:delivered:line:<part_index>`. Each part contains `company_id`, `delivery_id`, `delivery_number`, delivery `revision`, `status`, `order_id`, `order_number`, `order_revision`, `customer_kind`, `customer_id`, `customer_code`, `reference`, `part_index`, `part_count` and one line in `lines`. Each line contains `line_number`, `order_line_number`, `product_id`, `product_code`, `description`, `unit`, `ordered_quantity` and delivered `quantity`. Base supplies the version, actor and event instant. Consumers deduplicate event identity and collect every part for the delivery revision. A delivered delivery cannot be cancelled, so there is no reversal event in this slice.

## Where to go next

- [Domain build and selling guide](../README.md)
- Platform `apps/base/events/README.md` for durable delivery, replay and subscriber contracts.
- Platform `apps/base/agent_api/README.md` for preview and commit.
