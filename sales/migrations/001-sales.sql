-- Company-owned catalog and immutable document snapshots. A document changes
-- only by adding a status event; catalog edits never rewrite sold lines.
CREATE TABLE commerce_sales_items (
 id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
 code varchar(50) NOT NULL, revision bigint NOT NULL DEFAULT 1,
 facts jsonb NOT NULL CHECK (jsonb_typeof(facts) = 'object'),
 UNIQUE (tenant_id, company_id, code)
);
CREATE INDEX ON commerce_sales_items(tenant_id, company_id, id);
CREATE TABLE commerce_sales_documents (
 id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
 kind text NOT NULL CHECK (kind IN ('quotation','order')),
 number bigint NOT NULL, revision bigint NOT NULL DEFAULT 1,
 status text NOT NULL CHECK (status IN ('draft','sent','accepted','rejected','expired','confirmed','cancelled')),
 quotation_id bigint REFERENCES commerce_sales_documents(id),
 facts jsonb NOT NULL CHECK (jsonb_typeof(facts) = 'object'),
 UNIQUE (tenant_id, company_id, kind, number),
 UNIQUE (quotation_id)
);
CREATE INDEX ON commerce_sales_documents(tenant_id, company_id, kind, id);
CREATE TABLE commerce_sales_history (
 id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL,
 document_id bigint NOT NULL REFERENCES commerce_sales_documents(id),
 from_status text, to_status text NOT NULL,
 user_id bigint NOT NULL, impersonator_id bigint, reason text NOT NULL,
 recorded_at timestamp NOT NULL DEFAULT (now() AT TIME ZONE 'UTC')
);
CREATE INDEX ON commerce_sales_history(tenant_id,company_id,document_id,id);
CREATE FUNCTION commerce_sales_keep_history() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 RAISE EXCEPTION 'Commerce keeps document snapshots and status history' USING ERRCODE = 'restrict_violation';
END $$;
CREATE TRIGGER commerce_sales_history_kept BEFORE UPDATE OR DELETE ON commerce_sales_history
 FOR EACH ROW EXECUTE FUNCTION commerce_sales_keep_history();
CREATE TRIGGER commerce_sales_history_no_truncate BEFORE TRUNCATE ON commerce_sales_history
 FOR EACH STATEMENT EXECUTE FUNCTION commerce_sales_keep_history();
CREATE TRIGGER commerce_sales_documents_kept BEFORE DELETE ON commerce_sales_documents
 FOR EACH ROW EXECUTE FUNCTION commerce_sales_keep_history();
CREATE TRIGGER commerce_sales_documents_no_truncate BEFORE TRUNCATE ON commerce_sales_documents
 FOR EACH STATEMENT EXECUTE FUNCTION commerce_sales_keep_history();
CREATE FUNCTION commerce_sales_keep_snapshot() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF (NEW.id,NEW.tenant_id,NEW.company_id,NEW.kind,NEW.number,NEW.quotation_id,NEW.facts)
 IS DISTINCT FROM (OLD.id,OLD.tenant_id,OLD.company_id,OLD.kind,OLD.number,OLD.quotation_id,OLD.facts) THEN
 RAISE EXCEPTION 'Commerce document facts are immutable' USING ERRCODE = 'restrict_violation';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER commerce_sales_document_snapshot BEFORE UPDATE ON commerce_sales_documents
 FOR EACH ROW EXECUTE FUNCTION commerce_sales_keep_snapshot();
