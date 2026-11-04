# Sales contracts

The `commerce/sales` module owns catalog items, quotations, sales orders and their status history.

## Writes

Browser actions and agents call the same registered safe-write operations. The browser uses `PageContracts`; an agent previews the operation, gets the person's approval, then commits the unchanged input. Every operation authorizes the explicit `company_id` under the sealed scope, locks that company's writable row, validates the facts, and writes through `MutationBoundary`.

| Operation | Required facts beyond `company_id` |
| --- | --- |
| `commerce.sales.item.create` | `code`, `name`, `unit`, `default_price`, `tax_code`, `tax_rate` |
| `commerce.sales.item.update` | `id`, current `revision`, `name`, `unit`, `default_price`, `tax_code`, `tax_rate`; optional `active` |
| `commerce.sales.quotation.create` | `customer_kind`, `customer_code`, `currency_code`, `valid_until`, `lines`; optional `reference` |
| `commerce.sales.quotation.transition` | `id`, current `revision`, `status`; `reason` for rejection |
| `commerce.sales.order.create` | Either `quotation_id` (or `quotation_number`) or the same customer, currency and lines as a quotation; no expiry required |
| `commerce.sales.order.transition` | `id`, current `revision`, `status`; `reason` for cancellation |

`customer_kind` is `company` or `person`. These are Core record kinds, with the same meaning as Finance receivables. A typed line has `description`, `quantity`, `unit`, `unit_price`, `tax_code` and `tax_rate`. A catalog line instead has `item_id` or `item_code`, and `quantity`. The stored line includes all resolved facts, its 1-based `line_number`, and subtotal, tax and total.

Quantities, prices and rates are exact decimal strings with at most four decimal places and values at most 999,999,999. Quantities must be positive; prices and tax may be zero. Rates are percentages from 0 to 100. A document has 1–100 lines. Each line subtotal is quantity times price, rounded half up to two decimals. Tax is that rounded subtotal times the rate, rounded half up to two decimals. Document totals add those rounded amounts. Currency codes are three uppercase letters; Commerce performs no conversion.

Quotations move `draft → sent → accepted | rejected | expired`. Sales orders move `draft → confirmed → cancelled`, and drafts may be cancelled. A status change requires the current revision, so stale browser or agent actions are refused. Status history records creation and each transition with account, impersonator, reason and UTC instant. History and document facts are protected against changes and deletion by database triggers.

`commerce.sales.view` opens read pages. `commerce.sales.manage` permits writes; confirmation also requires `commerce.sales.confirm`. Tenant Owner receives all three through the vocabulary contribution. The company desk proof still applies to each call.

## Events for invoice consumers

Commerce declares two Base event types, each at **version 1**:

- `commerce/sales.order_confirmed`: an order moved from draft to confirmed.
- `commerce/sales.order_status_changed`: an order was confirmed or cancelled.

Publication happens on the order's audited transaction. Refusal or publication failure rolls back the complete write. There is no Finance dependency, port, SQL reference or invoice write.

Each event carries one line per part to stay within the Base event payload limits. The subject is `order:<id>` and the idempotency key is `revision:<revision>:line:<line_number>`. The Base event envelope supplies tenant, event type version, actor and event time. The payload contains:

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
  "currency_code": "ABC",
  "subtotal": "10.00",
  "tax_total": "0.00",
  "total": "10.00",
  "part_index": 1,
  "part_count": 1,
  "lines": [{
    "line_number": 1,
    "item_id": null,
    "item_code": null,
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

## Where to go next

- [Domain build and selling guide](../README.md)
- Platform `apps/base/events/README.md` for durable delivery, replay and subscriber contracts.
- Platform `apps/base/agent_api/README.md` for preview and commit.
