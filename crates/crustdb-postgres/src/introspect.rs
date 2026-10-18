//! Catalog introspection. Every name filter is a bound text parameter; the grid-path
//! session returns each column as text, so all selected values are cast with `::text`.
use async_trait::async_trait;
use crustdb_core::dialect::Dialect;
use crustdb_core::driver::{Introspector, Param, Session};
use crustdb_core::error::DriverError;
use crustdb_core::model::{
    Cell, ColumnDetail, ColumnMeta, ForeignKeyInfo, IndexInfo, ObjectKind, ObjectRef,
};

use crate::PgDialect;
use crate::connect::err;
use crate::types::category_for_oid;
use crustdb_core::error::ErrorKind;

type Rows = Vec<Vec<Option<String>>>;

async fn query_rows(
    s: &mut dyn Session,
    sql: &str,
    params: &[Option<&str>],
) -> Result<Rows, DriverError> {
    let params: Vec<_> = params.iter().map(|p| Param(p.map(String::from))).collect();
    s.start_params(sql, &params).await?;
    let mut rows = Rows::new();
    loop {
        let f = match s.fetch(1000).await {
            Ok(f) => f,
            Err(e) => {
                let _ = s.finish().await;
                return Err(e);
            }
        };
        rows.extend(f.rows.iter().map(|r| {
            r.iter()
                .map(|c| match c {
                    Cell::Text { value, .. } => Some(value.to_string()),
                    _ => None,
                })
                .collect()
        }));
        if f.exhausted {
            break;
        }
    }
    s.finish().await?;
    Ok(rows)
}

fn col(r: &[Option<String>], i: usize) -> String {
    r.get(i).cloned().flatten().unwrap_or_default()
}

fn is_true(r: &[Option<String>], i: usize) -> bool {
    col(r, i) == "true"
}

fn schema_of(o: &ObjectRef) -> &str {
    o.schema.as_deref().unwrap_or("public")
}

const NOT_HIDDEN_SCHEMA: &str = "n.nspname <> 'pg_toast' AND n.nspname NOT LIKE 'pg\\_temp\\_%' \
     AND n.nspname NOT LIKE 'pg\\_toast\\_temp\\_%' AND ($1::text IS NULL OR n.nspname = $1::text)";

/// `$1` = schema, `$2` = relation name.
const REL_OID: &str = "(SELECT c.oid FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
     WHERE n.nspname = $1 AND c.relname = $2)";

fn objects_sql(kind: ObjectKind) -> Option<String> {
    let rel = |relkinds: &str| {
        format!(
            "SELECT DISTINCT n.nspname::text, c.relname::text FROM pg_class c \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE c.relkind::text IN ({relkinds}) AND {NOT_HIDDEN_SCHEMA} ORDER BY 2"
        )
    };
    let proc = |prokind: &str| {
        format!(
            "SELECT DISTINCT n.nspname::text, p.proname::text FROM pg_proc p \
             JOIN pg_namespace n ON n.oid = p.pronamespace \
             WHERE p.prokind::text = '{prokind}' AND {NOT_HIDDEN_SCHEMA} ORDER BY 2"
        )
    };
    Some(match kind {
        ObjectKind::Table => rel("'r', 'p'"),
        ObjectKind::View => rel("'v'"),
        ObjectKind::MaterializedView => rel("'m'"),
        ObjectKind::Sequence => rel("'S'"),
        ObjectKind::Function => proc("f"),
        ObjectKind::Procedure => proc("p"),
        ObjectKind::Trigger => format!(
            "SELECT DISTINCT n.nspname::text, t.tgname::text FROM pg_trigger t \
             JOIN pg_class c ON c.oid = t.tgrelid JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE NOT t.tgisinternal AND {NOT_HIDDEN_SCHEMA} ORDER BY 2"
        ),
        ObjectKind::Type => format!(
            "SELECT n.nspname::text, t.typname::text FROM pg_type t \
             JOIN pg_namespace n ON n.oid = t.typnamespace \
             LEFT JOIN pg_class c ON c.oid = t.typrelid \
             WHERE (t.typtype::text IN ('e', 'd') OR (t.typtype::text = 'c' AND c.relkind::text = 'c')) \
             AND {NOT_HIDDEN_SCHEMA} ORDER BY 2"
        ),
        ObjectKind::Database | ObjectKind::Schema => return None,
    })
}

fn fk_action(c: &str) -> &'static str {
    match c {
        "r" => "RESTRICT",
        "c" => "CASCADE",
        "n" => "SET NULL",
        "d" => "SET DEFAULT",
        _ => "NO ACTION",
    }
}

fn unsupported_kind(k: ObjectKind) -> DriverError {
    err(
        ErrorKind::Unsupported,
        format!("no catalog query for {k:?}"),
    )
}

pub struct PgIntrospector;

impl PgIntrospector {
    async fn table_ddl(&self, s: &mut dyn Session, o: &ObjectRef) -> Result<String, DriverError> {
        let (sch, name) = (Some(schema_of(o)), Some(o.name.as_str()));
        let d = PgDialect;
        let meta = query_rows(
            s,
            &format!(
                "SELECT c.relkind::text, c.relispartition::text, \
                        CASE WHEN c.relkind::text = 'p' THEN pg_get_partkeydef(c.oid) END, \
                        (SELECT pn.nspname::text || '.' || pc.relname::text FROM pg_inherits h \
                           JOIN pg_class pc ON pc.oid = h.inhparent \
                           JOIN pg_namespace pn ON pn.oid = pc.relnamespace WHERE h.inhrelid = c.oid), \
                        pg_get_expr(c.relpartbound, c.oid) \
                 FROM pg_class c WHERE c.oid = {REL_OID}"
            ),
            &[sch, name],
        )
        .await?;
        if let Some(m) = meta.first().filter(|m| is_true(m, 1)) {
            // Partition: only the bound; columns come from the parent.
            let parent = col(m, 3);
            let (ps, pn) = parent.split_once('.').unwrap_or(("public", &parent));
            let parent = d.qualified_name(&ObjectRef {
                schema: Some(ps.into()),
                name: pn.into(),
                ..o.clone()
            });
            return Ok(format!(
                "CREATE TABLE {} PARTITION OF {parent} {};",
                d.qualified_name(o),
                col(m, 4)
            ));
        }
        let partition_by = meta.first().and_then(|m| m[2].clone());
        let cols: Vec<_> = self
            .columns(s, o)
            .await?
            .into_iter()
            .map(|c| {
                let mut l = format!("    {} {}", d.quote_ident(&c.meta.name), c.meta.type_name);
                if c.meta.nullable == Some(false) {
                    l.push_str(" NOT NULL");
                }
                if let Some(def) = c.default {
                    l.push_str(&format!(" DEFAULT {def}"));
                }
                l
            })
            .collect();
        let cons = query_rows(
            s,
            &format!(
                "SELECT '    CONSTRAINT ' || quote_ident(conname) || ' ' || pg_get_constraintdef(oid) \
                 FROM pg_constraint WHERE conrelid = {REL_OID} ORDER BY contype DESC, conname"
            ),
            &[sch, name],
        )
        .await?;
        let lines: Vec<_> = cols
            .into_iter()
            .chain(cons.iter().map(|r| col(r, 0)))
            .collect();
        let mut ddl = format!(
            "CREATE TABLE {} (\n{}\n);",
            d.qualified_name(o),
            lines.join(",\n")
        );
        if let Some(pb) = partition_by {
            ddl.insert_str(ddl.len() - 1, &format!(" PARTITION BY {pb}"));
        }
        let idx = query_rows(
            s,
            &format!(
                "SELECT pg_get_indexdef(i.indexrelid) || ';' FROM pg_index i \
                 WHERE i.indrelid = {REL_OID} AND NOT EXISTS \
                 (SELECT 1 FROM pg_constraint k WHERE k.conindid = i.indexrelid AND k.conrelid = i.indrelid) \
                 ORDER BY 1"
            ),
            &[sch, name],
        )
        .await?;
        for r in &idx {
            ddl.push_str(&format!("\n{}", col(r, 0)));
        }
        Ok(ddl)
    }
}

#[async_trait]
impl Introspector for PgIntrospector {
    async fn databases(&self, s: &mut dyn Session) -> Result<Vec<String>, DriverError> {
        let r = query_rows(
            s,
            "SELECT datname::text FROM pg_database WHERE NOT datistemplate AND datallowconn ORDER BY 1",
            &[],
        )
        .await?;
        Ok(r.iter().map(|r| col(r, 0)).collect())
    }

    async fn schemas(&self, s: &mut dyn Session, _: &str) -> Result<Vec<String>, DriverError> {
        let r = query_rows(
            s,
            &format!(
                "SELECT n.nspname::text FROM pg_namespace n WHERE {NOT_HIDDEN_SCHEMA} ORDER BY 1"
            ),
            &[None],
        )
        .await?;
        Ok(r.iter().map(|r| col(r, 0)).collect())
    }

    // The session is already connected to `database`; it only fills ObjectRef.database.
    async fn objects(
        &self,
        s: &mut dyn Session,
        database: &str,
        schema: Option<&str>,
        kind: ObjectKind,
    ) -> Result<Vec<ObjectRef>, DriverError> {
        let sql = objects_sql(kind).ok_or_else(|| unsupported_kind(kind))?;
        let r = query_rows(s, &sql, &[schema]).await?;
        Ok(r.iter()
            .map(|r| ObjectRef {
                database: database.into(),
                schema: Some(col(r, 0)),
                name: col(r, 1),
                kind,
            })
            .collect())
    }

    async fn columns(
        &self,
        s: &mut dyn Session,
        o: &ObjectRef,
    ) -> Result<Vec<ColumnDetail>, DriverError> {
        let r = query_rows(
            s,
            "SELECT a.attname::text, format_type(a.atttypid, a.atttypmod), a.atttypid::text, \
                    (NOT a.attnotnull)::text, pg_get_expr(d.adbin, d.adrelid), \
                    col_description(c.oid, a.attnum), \
                    EXISTS (SELECT 1 FROM pg_index i WHERE i.indrelid = c.oid AND i.indisprimary \
                            AND a.attnum = ANY (i.indkey))::text \
             FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
             WHERE n.nspname = $1 AND c.relname = $2 AND a.attnum > 0 AND NOT a.attisdropped \
             ORDER BY a.attnum",
            &[Some(schema_of(o)), Some(&o.name)],
        )
        .await?;
        Ok(r.iter()
            .map(|r| ColumnDetail {
                meta: ColumnMeta {
                    name: col(r, 0),
                    // format_type output is quoted/qualified; PgDialect::param interpolates it.
                    type_name: col(r, 1),
                    category: category_for_oid(col(r, 2).parse().unwrap_or(0)),
                    nullable: Some(is_true(r, 3)),
                    is_primary_key: is_true(r, 6),
                },
                default: r[4].clone(),
                comment: r[5].clone(),
            })
            .collect())
    }

    async fn indexes(
        &self,
        s: &mut dyn Session,
        o: &ObjectRef,
    ) -> Result<Vec<IndexInfo>, DriverError> {
        let r = query_rows(
            s,
            "SELECT ic.relname::text, i.indisunique::text, i.indisprimary::text, am.amname::text, \
                    coalesce(a.attname::text, pg_get_indexdef(i.indexrelid, k.ord::int, true)) \
             FROM pg_index i JOIN pg_class ic ON ic.oid = i.indexrelid \
             JOIN pg_class c ON c.oid = i.indrelid JOIN pg_namespace n ON n.oid = c.relnamespace \
             JOIN pg_am am ON am.oid = ic.relam \
             CROSS JOIN LATERAL unnest(i.indkey::int2[]) WITH ORDINALITY AS k(attnum, ord) \
             LEFT JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = k.attnum \
             WHERE n.nspname = $1 AND c.relname = $2 ORDER BY ic.relname, k.ord",
            &[Some(schema_of(o)), Some(&o.name)],
        )
        .await?;
        let mut out: Vec<IndexInfo> = Vec::new();
        for r in &r {
            let name = col(r, 0);
            if out.last().is_none_or(|l| l.name != name) {
                out.push(IndexInfo {
                    name,
                    columns: vec![],
                    unique: is_true(r, 1),
                    primary: is_true(r, 2),
                    method: r[3].clone(),
                });
            }
            if let Some(l) = out.last_mut() {
                l.columns.push(col(r, 4));
            }
        }
        Ok(out)
    }

    async fn foreign_keys(
        &self,
        s: &mut dyn Session,
        o: &ObjectRef,
    ) -> Result<Vec<ForeignKeyInfo>, DriverError> {
        let r = query_rows(
            s,
            "SELECT con.conname::text, con.confupdtype::text, con.confdeltype::text, \
                    rn.nspname::text, rc.relname::text, fa.attname::text, ra.attname::text \
             FROM pg_constraint con JOIN pg_class c ON c.oid = con.conrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             JOIN pg_class rc ON rc.oid = con.confrelid \
             JOIN pg_namespace rn ON rn.oid = rc.relnamespace \
             CROSS JOIN LATERAL unnest(con.conkey, con.confkey) WITH ORDINALITY AS k(l, r, ord) \
             JOIN pg_attribute fa ON fa.attrelid = con.conrelid AND fa.attnum = k.l \
             JOIN pg_attribute ra ON ra.attrelid = con.confrelid AND ra.attnum = k.r \
             WHERE con.contype::text = 'f' AND n.nspname = $1 AND c.relname = $2 \
             ORDER BY con.conname, k.ord",
            &[Some(schema_of(o)), Some(&o.name)],
        )
        .await?;
        let mut out: Vec<ForeignKeyInfo> = Vec::new();
        for r in &r {
            let name = col(r, 0);
            if out.last().is_none_or(|l| l.name != name) {
                out.push(ForeignKeyInfo {
                    name,
                    columns: vec![],
                    ref_table: ObjectRef {
                        database: o.database.clone(),
                        schema: Some(col(r, 3)),
                        name: col(r, 4),
                        kind: ObjectKind::Table,
                    },
                    ref_columns: vec![],
                    on_update: fk_action(&col(r, 1)).into(),
                    on_delete: fk_action(&col(r, 2)).into(),
                });
            }
            if let Some(l) = out.last_mut() {
                l.columns.push(col(r, 5));
                l.ref_columns.push(col(r, 6));
            }
        }
        Ok(out)
    }

    async fn ddl(&self, s: &mut dyn Session, o: &ObjectRef) -> Result<String, DriverError> {
        let (sch, name) = (Some(schema_of(o)), Some(o.name.as_str()));
        let q = PgDialect.qualified_name(o);
        let sql = match o.kind {
            ObjectKind::Table => return self.table_ddl(s, o).await,
            ObjectKind::View | ObjectKind::MaterializedView => {
                let r = query_rows(
                    s,
                    &format!("SELECT pg_get_viewdef({REL_OID}, true)"),
                    &[sch, name],
                )
                .await?;
                let m = if o.kind == ObjectKind::View {
                    ""
                } else {
                    "MATERIALIZED "
                };
                let body = r.first().map(|r| col(r, 0)).unwrap_or_default();
                return Ok(format!("CREATE {m}VIEW {q} AS\n{}", body.trim_end()));
            }
            ObjectKind::Function => {
                "SELECT pg_get_functiondef(p.oid) FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
                 WHERE n.nspname = $1 AND p.proname = $2 AND p.prokind::text = 'f' ORDER BY p.oid"
            }
            ObjectKind::Procedure => {
                "SELECT pg_get_functiondef(p.oid) FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
                 WHERE n.nspname = $1 AND p.proname = $2 AND p.prokind::text = 'p' ORDER BY p.oid"
            }
            // ponytail: same-named triggers on different tables in one schema are ambiguous; first match wins.
            ObjectKind::Trigger => {
                "SELECT pg_get_triggerdef(t.oid) FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname = $1 AND t.tgname = $2 AND NOT t.tgisinternal ORDER BY t.oid LIMIT 1"
            }
            ObjectKind::Sequence => {
                "SELECT format('CREATE SEQUENCE %I.%I AS %s START WITH %s INCREMENT BY %s MINVALUE %s \
                 MAXVALUE %s CACHE %s%s;', schemaname, sequencename, data_type::text, start_value, \
                 increment_by, min_value, max_value, cache_size, CASE WHEN cycle THEN ' CYCLE' ELSE '' END) \
                 FROM pg_sequences WHERE schemaname = $1 AND sequencename = $2"
            }
            ObjectKind::Type => {
                "SELECT CASE t.typtype::text \
                   WHEN 'e' THEN format('CREATE TYPE %I.%I AS ENUM (%s);', n.nspname, t.typname, \
                     (SELECT string_agg(quote_literal(e.enumlabel), ', ' ORDER BY e.enumsortorder) \
                      FROM pg_enum e WHERE e.enumtypid = t.oid)) \
                   WHEN 'c' THEN format('CREATE TYPE %I.%I AS (%s);', n.nspname, t.typname, \
                     (SELECT string_agg(quote_ident(a.attname) || ' ' || format_type(a.atttypid, a.atttypmod), \
                             ', ' ORDER BY a.attnum) \
                      FROM pg_attribute a WHERE a.attrelid = t.typrelid AND a.attnum > 0 AND NOT a.attisdropped)) \
                   ELSE format('CREATE DOMAIN %I.%I AS %s%s%s%s;', n.nspname, t.typname, \
                     format_type(t.typbasetype, t.typtypmod), coalesce(' DEFAULT ' || t.typdefault, ''), \
                     CASE WHEN t.typnotnull THEN ' NOT NULL' ELSE '' END, \
                     coalesce((SELECT string_agg(' ' || pg_get_constraintdef(k.oid), '') \
                               FROM pg_constraint k WHERE k.contypid = t.oid), '')) \
                 END FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace \
                 WHERE n.nspname = $1 AND t.typname = $2"
            }
            ObjectKind::Database | ObjectKind::Schema => return Err(unsupported_kind(o.kind)),
        };
        let r = query_rows(s, sql, &[sch, name]).await?;
        // Overloads: every definition, separated by a blank line.
        Ok(r.iter().map(|r| col(r, 0)).collect::<Vec<_>>().join("\n\n"))
    }

    async fn row_estimate(
        &self,
        s: &mut dyn Session,
        o: &ObjectRef,
    ) -> Result<Option<u64>, DriverError> {
        if o.kind != ObjectKind::Table {
            return Ok(None);
        }
        let r = query_rows(
            s,
            &format!(
                "SELECT reltuples::float8::text, relpages::text FROM pg_class WHERE oid = {REL_OID}"
            ),
            &[Some(schema_of(o)), Some(&o.name)],
        )
        .await?;
        let Some(r) = r.first() else { return Ok(None) };
        let tuples: f64 = col(r, 0).parse().unwrap_or(-1.0);
        // PG < 14 reports 0/0 (not -1) for a never-analyzed table.
        if tuples < 0.0 || (tuples == 0.0 && col(r, 1) == "0") {
            return Ok(None);
        }
        Ok(Some(tuples as u64))
    }
}
