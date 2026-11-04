use crate::Record;
use bilibi_agent_api::OperationClient;
use bilibi_database::{Change, MutationTransaction, Result};
use bilibi_tenancy::Scope;
use serde_json::{json, Value};
use tokio_postgres::Row;

fn row(r: &Row, kind: &str) -> Result<Record> {
    Ok(Record {
        id: r.try_get(0)?,
        revision: r.try_get(1)?,
        code: r.try_get(2)?,
        status: r.try_get(3)?,
        facts: r.try_get(4)?,
        kind: kind.into(),
    })
}
pub(crate) async fn get(
    c: &dyn OperationClient,
    t: i64,
    co: i64,
    k: &str,
    id: i64,
) -> Result<Option<Record>> {
    let sql = if k == "item" {
        "SELECT id,revision,code,'active',facts FROM commerce_sales_items WHERE tenant_id=$1 AND company_id=$2 AND id=$3"
    } else {
        "SELECT id,revision,number::text,status,facts FROM commerce_sales_documents WHERE tenant_id=$1 AND company_id=$2 AND id=$3 AND kind=$4"
    };
    let r = if k == "item" {
        c.query_opt(sql, &[&t, &co, &id]).await?
    } else {
        c.query_opt(sql, &[&t, &co, &id, &k]).await?
    };
    r.as_ref().map(|r| row(r, k)).transpose()
}
#[allow(clippy::too_many_arguments)]
pub(crate) async fn list(
    c: &dyn OperationClient,
    t: i64,
    co: i64,
    k: &str,
    q: &str,
    status: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<Record>> {
    let query = format!("%{}%", q.replace('%', "\\%").replace('_', "\\_"));
    let rows = if k == "item" {
        c.query("SELECT id,revision,code,'active',facts FROM commerce_sales_items WHERE tenant_id=$1 AND company_id=$2 AND (code ILIKE $3 OR facts->>'name' ILIKE $3) ORDER BY id DESC LIMIT $4 OFFSET $5",&[&t,&co,&query,&limit,&offset]).await?
    } else {
        c.query("SELECT id,revision,number::text,status,facts FROM commerce_sales_documents WHERE tenant_id=$1 AND company_id=$2 AND kind=$3 AND ($4='' OR status=$4) AND (number::text ILIKE $5 OR facts->>'customer_code' ILIKE $5) ORDER BY id DESC LIMIT $6 OFFSET $7",&[&t,&co,&k,&status,&query,&limit,&offset]).await?
    };
    rows.iter().map(|r| row(r, k)).collect()
}
pub(crate) async fn history(
    c: &dyn OperationClient,
    t: i64,
    co: i64,
    id: i64,
) -> Result<Vec<Value>> {
    c.query("SELECT id,from_status,to_status,user_id,impersonator_id,reason,recorded_at FROM commerce_sales_history WHERE tenant_id=$1 AND company_id=$2 AND document_id=$3 ORDER BY id",&[&t,&co,&id]).await?.iter().map(|r|Ok(json!({"id":r.try_get::<_,i64>(0)?,"from":r.try_get::<_,Option<String>>(1)?,"to":r.try_get::<_,String>(2)?,"user_id":r.try_get::<_,i64>(3)?,"impersonator_id":r.try_get::<_,Option<i64>>(4)?,"reason":r.try_get::<_,String>(5)?,"at":jiff::Timestamp::try_from(r.try_get::<_,std::time::SystemTime>(6)?).map_err(|_|bilibi_database::Error::Internal)?.to_string()}))).collect()
}

#[allow(clippy::too_many_arguments)] // One audited write names its complete prepared snapshot.
pub(crate) async fn apply(
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    co: i64,
    kind: &str,
    code: &str,
    old: Option<Record>,
    facts: Value,
    target: &str,
    reason: &str,
    quotation: Option<i64>,
) -> Result<Record> {
    let t = scope.tenant();
    let user = scope.user();
    let impersonator = scope.impersonator().map(|operator| operator.user());
    let kind = kind.to_owned();
    let code = code.to_owned();
    let target = target.to_owned();
    let reason = reason.to_owned();
    tx.mutate(move |sql|Box::pin(async move {
        let r=if kind=="item" {
            if let Some(before)=&old { sql.query_one("UPDATE commerce_sales_items SET facts=$4,revision=revision+1 WHERE tenant_id=$1 AND company_id=$2 AND id=$3 RETURNING id,revision,code,'active',facts",&[&t,&co,&before.id,&facts]).await? }
            else { sql.query_one("INSERT INTO commerce_sales_items(tenant_id,company_id,code,facts) VALUES($1,$2,$3,$4) RETURNING id,revision,code,'active',facts",&[&t,&co,&code,&facts]).await? }
        } else if let Some(before)=&old {
            sql.query_one("UPDATE commerce_sales_documents SET status=$4,revision=revision+1 WHERE tenant_id=$1 AND company_id=$2 AND id=$3 RETURNING id,revision,number::text,status,facts",&[&t,&co,&before.id,&target]).await?
        } else {
            sql.query_one("INSERT INTO commerce_sales_documents(tenant_id,company_id,kind,number,status,quotation_id,facts) SELECT $1,$2,$3,COALESCE(MAX(number),0)+1,$4,$5,$6 FROM commerce_sales_documents WHERE tenant_id=$1 AND company_id=$2 AND kind=$3 RETURNING id,revision,number::text,status,facts",&[&t,&co,&kind,&target,&quotation,&facts]).await?
        };
        let saved=row(&r,&kind)?;
        let mut changes=vec![Change{kind:format!("commerce.sales.{kind}"),id:saved.id,old:old.as_ref().map_or(json!({}),Record::snapshot),new:saved.snapshot()}];
        if kind!="item" {
            let from=old.as_ref().map(|r|r.status.clone());
            let h=sql.query_one("INSERT INTO commerce_sales_history(tenant_id,company_id,document_id,from_status,to_status,user_id,impersonator_id,reason) VALUES($1,$2,$3,$4,$5,$6,$7,$8) RETURNING id",&[&t,&co,&saved.id,&from,&target,&user,&impersonator,&reason]).await?;
            changes.push(Change{kind:"commerce.sales.history".into(),id:h.try_get(0)?,old:json!({}),new:json!({"document_id":saved.id,"from_status":from,"to_status":target,"user_id":user,"impersonator_id":impersonator,"reason":reason})});
        }
        Ok((saved,changes))
    })).await
}
pub(crate) async fn count(
    c: &dyn OperationClient,
    t: i64,
    co: i64,
    k: &str,
    q: &str,
    status: &str,
) -> Result<i64> {
    let query = format!("%{}%", q.replace('%', "\\%").replace('_', "\\_"));
    let row = if k == "item" {
        c.query_opt("SELECT COUNT(*) FROM commerce_sales_items WHERE tenant_id=$1 AND company_id=$2 AND (code ILIKE $3 OR facts->>'name' ILIKE $3)",&[&t,&co,&query]).await?
    } else {
        c.query_opt("SELECT COUNT(*) FROM commerce_sales_documents WHERE tenant_id=$1 AND company_id=$2 AND kind=$3 AND ($4='' OR status=$4) AND (number::text ILIKE $5 OR facts->>'customer_code' ILIKE $5)",&[&t,&co,&k,&status,&query]).await?
    };
    Ok(row.ok_or(bilibi_database::Error::Internal)?.try_get(0)?)
}
