-- Delivery snapshots share document numbering, audit and immutable history.
ALTER TABLE commerce_sales_documents DROP CONSTRAINT commerce_sales_documents_kind_check;
ALTER TABLE commerce_sales_documents ADD CHECK (kind IN ('quotation','order','delivery'));
ALTER TABLE commerce_sales_documents DROP CONSTRAINT commerce_sales_documents_status_check;
ALTER TABLE commerce_sales_documents ADD CHECK (status IN ('draft','sent','accepted','rejected','expired','confirmed','shipped','delivered','cancelled'));
CREATE INDEX commerce_sales_delivery_order ON commerce_sales_documents(tenant_id,company_id,((facts->>'order_id')::bigint)) WHERE kind='delivery';
