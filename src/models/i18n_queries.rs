use std::collections::HashMap;

use sea_orm::{
    sea_query::{Alias, Expr, Func, Order as SqOrder, Query as SqQuery},
    ColumnTrait, Condition, ConnectionTrait, DatabaseConnection, DbErr, EntityTrait,
    PaginatorTrait, QueryFilter, QueryOrder, Statement,
};
use uuid::Uuid;

use crate::models::_entities::{i18n_entries, i18n_entry_locations, i18n_translations};
use crate::models::i18n_bundle_revisions as rev_model;

/// Scoping for key-listing queries. Eliminates the runtime panic risk of
/// `include_global=true + tenant_id=None`.
#[derive(Debug, Clone, Copy)]
pub enum KeyScope {
    /// Only rows where `tenant_id IS NULL` (global scope).
    GlobalOnly,
    /// Only rows where `tenant_id = $1` (single tenant scope).
    TenantOnly(Uuid),
    /// Union: rows where `tenant_id IS NULL OR tenant_id = $1` (tenant sees
    /// its own overrides plus inherited globals).
    TenantWithGlobal(Uuid),
}

#[derive(Debug, Clone)]
pub struct NamespaceCount {
    pub namespace: String,
    pub key_count: u64,
    pub locale_count: u64,
}

pub async fn read_revisions(
    db: &DatabaseConnection,
    locale: &str,
    namespace: &str,
    tenant_id: Option<Uuid>,
) -> Result<(i64, i64), DbErr> {
    let global_rev = rev_model::Model::find_global(db, locale, namespace)
        .await?
        .map_or(0, |r| r.revision);

    let tenant_rev = if let Some(tid) = tenant_id {
        rev_model::Model::find_tenant(db, locale, namespace, tid)
            .await?
            .map_or(0, |r| r.revision)
    } else {
        0
    };

    Ok((global_rev, tenant_rev))
}

pub async fn list_namespaces(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
) -> Result<Vec<NamespaceCount>, DbErr> {
    let col_ns = i18n_translations::Column::Namespace;
    let col_key = i18n_translations::Column::Key;
    let col_locale = i18n_translations::Column::Locale;
    let col_tid = i18n_translations::Column::TenantId;

    let mut query = SqQuery::select()
        .column(col_ns)
        .expr_as(
            Func::count_distinct(Expr::col(col_key)),
            Alias::new("key_count"),
        )
        .expr_as(
            Func::count_distinct(Expr::col(col_locale)),
            Alias::new("locale_count"),
        )
        .from(i18n_translations::Entity)
        .group_by_col(col_ns)
        .order_by(col_ns, SqOrder::Asc)
        .to_owned();

    match tenant_id {
        Some(tid) => {
            query.and_where(Expr::col(col_tid).eq(tid));
        }
        None => {
            query.and_where(Expr::col(col_tid).is_null());
        }
    };

    let builder = db.get_database_backend();
    let rows = db.query_all(builder.build(&query)).await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let namespace: String = row.try_get("", "namespace")?;
        let key_count: i64 = row.try_get("", "key_count")?;
        let locale_count: i64 = row.try_get("", "locale_count")?;
        out.push(NamespaceCount {
            namespace,
            key_count: key_count.max(0) as u64,
            locale_count: locale_count.max(0) as u64,
        });
    }
    Ok(out)
}

pub async fn list_tenant_namespaces(
    db: &DatabaseConnection,
    tenant_id: Uuid,
) -> Result<Vec<NamespaceCount>, DbErr> {
    let col_ns = i18n_translations::Column::Namespace;
    let col_key = i18n_translations::Column::Key;
    let col_locale = i18n_translations::Column::Locale;
    let col_tid = i18n_translations::Column::TenantId;

    let query = SqQuery::select()
        .column(col_ns)
        .expr_as(
            Func::count_distinct(Expr::col(col_key)),
            Alias::new("key_count"),
        )
        .expr_as(
            Func::count_distinct(Expr::col(col_locale)),
            Alias::new("locale_count"),
        )
        .from(i18n_translations::Entity)
        .and_where(
            Expr::col(col_tid)
                .is_null()
                .or(Expr::col(col_tid).eq(tenant_id)),
        )
        .group_by_col(col_ns)
        .order_by(col_ns, SqOrder::Asc)
        .to_owned();

    let builder = db.get_database_backend();
    let rows = db.query_all(builder.build(&query)).await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let namespace: String = row.try_get("", "namespace")?;
        let key_count: i64 = row.try_get("", "key_count")?;
        let locale_count: i64 = row.try_get("", "locale_count")?;
        out.push(NamespaceCount {
            namespace,
            key_count: key_count.max(0) as u64,
            locale_count: locale_count.max(0) as u64,
        });
    }
    Ok(out)
}

pub async fn export_global(
    db: &DatabaseConnection,
    namespace: Option<&str>,
    locale: Option<&str>,
) -> Result<Vec<i18n_translations::Model>, DbErr> {
    let mut query = i18n_translations::Entity::find()
        .filter(i18n_translations::Column::TenantId.is_null());
    if let Some(ns) = namespace {
        query = query.filter(i18n_translations::Column::Namespace.eq(ns));
    }
    if let Some(loc) = locale {
        query = query.filter(i18n_translations::Column::Locale.eq(loc));
    }

    query
        .order_by_asc(i18n_translations::Column::Namespace)
        .order_by_asc(i18n_translations::Column::Key)
        .order_by_asc(i18n_translations::Column::Locale)
        .all(db)
        .await
}

pub async fn export_tenant(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    namespace: Option<&str>,
    locale: Option<&str>,
) -> Result<Vec<i18n_translations::Model>, DbErr> {
    let mut query = i18n_translations::Entity::find()
        .filter(i18n_translations::Column::TenantId.eq(tenant_id));
    if let Some(ns) = namespace {
        query = query.filter(i18n_translations::Column::Namespace.eq(ns));
    }
    if let Some(loc) = locale {
        query = query.filter(i18n_translations::Column::Locale.eq(loc));
    }

    query
        .order_by_asc(i18n_translations::Column::Namespace)
        .order_by_asc(i18n_translations::Column::Key)
        .order_by_asc(i18n_translations::Column::Locale)
        .all(db)
        .await
}

pub async fn list_entries(
    db: &DatabaseConnection,
    namespace: Option<&str>,
    status: Option<&str>,
    page: u64,
    page_size: u64,
) -> Result<(Vec<i18n_entries::Model>, u64), DbErr> {
    let mut query = i18n_entries::Entity::find();
    if let Some(ns) = namespace {
        query = query.filter(i18n_entries::Column::Namespace.eq(ns));
    }
    if let Some(s) = status {
        query = query.filter(i18n_entries::Column::Status.eq(s));
    }

    let paginator = query
        .order_by_asc(i18n_entries::Column::Namespace)
        .order_by_asc(i18n_entries::Column::Key)
        .paginate(db, page_size);

    let total = paginator.num_items().await?;
    let rows = paginator.fetch_page(page.saturating_sub(1)).await?;
    Ok((rows, total))
}

pub async fn list_entry_locations(
    db: &DatabaseConnection,
    entry_id: Uuid,
) -> Result<Vec<i18n_entry_locations::Model>, DbErr> {
    i18n_entry_locations::Entity::find()
        .filter(i18n_entry_locations::Column::EntryId.eq(entry_id))
        .order_by_asc(i18n_entry_locations::Column::FilePath)
        .order_by_asc(i18n_entry_locations::Column::Line)
        .all(db)
        .await
}

/// Batch-fetch entry metadata for a set of `(namespace, key)` pairs.
/// Returns a HashMap keyed by `(namespace, key)` for O(1) lookup.
pub async fn fetch_entries_by_pairs(
    db: &DatabaseConnection,
    pairs: &[(String, String)],
) -> Result<HashMap<(String, String), i18n_entries::Model>, DbErr> {
    if pairs.is_empty() {
        return Ok(HashMap::new());
    }

    let mut filter = Condition::any();
    for (ns, key) in pairs {
        filter = filter.add(
            Condition::all()
                .add(i18n_entries::Column::Namespace.eq(ns.as_str()))
                .add(i18n_entries::Column::Key.eq(key.as_str())),
        );
    }

    let rows = i18n_entries::Entity::find().filter(filter).all(db).await?;

    Ok(rows
        .into_iter()
        .map(|row| ((row.namespace.clone(), row.key.clone()), row))
        .collect())
}

pub async fn count_distinct_keys(
    db: &DatabaseConnection,
    scope: KeyScope,
    namespace: Option<&str>,
    q: Option<&str>,
    empty_locale: Option<&str>,
) -> Result<u64, DbErr> {
    let backend = db.get_database_backend();
    let (where_sql, binds) =
        build_distinct_keys_where(&scope, namespace, q, empty_locale);
    let count_sql = format!(
        "SELECT COUNT(*) AS cnt FROM (SELECT DISTINCT namespace, key \
         FROM i18n_translations WHERE {where_sql}) AS sub"
    );

    let total: i64 = db
        .query_one(Statement::from_sql_and_values(backend, count_sql, binds))
        .await?
        .ok_or_else(|| DbErr::Custom("count query returned no row".into()))?
        .try_get("", "cnt")?;

    Ok(total.max(0) as u64)
}

pub async fn paginate_distinct_keys(
    db: &DatabaseConnection,
    scope: KeyScope,
    namespace: Option<&str>,
    q: Option<&str>,
    empty_locale: Option<&str>,
    page_size: u64,
    offset: u64,
) -> Result<Vec<(String, String)>, DbErr> {
    let backend = db.get_database_backend();
    let (where_sql, binds) =
        build_distinct_keys_where(&scope, namespace, q, empty_locale);
    let mut page_binds = binds;
    page_binds.push((page_size as i64).into());
    let limit_idx = page_binds.len();
    page_binds.push((offset as i64).into());
    let offset_idx = page_binds.len();
    let page_sql = format!(
        "SELECT DISTINCT namespace, key FROM i18n_translations \
         WHERE {where_sql} \
         ORDER BY namespace ASC, key ASC \
         LIMIT ${limit_idx} OFFSET ${offset_idx}"
    );

    let rows = db
        .query_all(Statement::from_sql_and_values(
            backend, page_sql, page_binds,
        ))
        .await?;

    rows.into_iter()
        .map(|row| Ok((row.try_get("", "namespace")?, row.try_get("", "key")?)))
        .collect()
}

pub async fn fetch_detail_rows(
    db: &DatabaseConnection,
    tenant_id: Option<Uuid>,
    pairs: &[(String, String)],
) -> Result<Vec<i18n_translations::Model>, DbErr> {
    if pairs.is_empty() {
        return Ok(Vec::new());
    }

    let mut query = i18n_translations::Entity::find();
    query = match tenant_id {
        Some(tid) => query.filter(i18n_translations::Column::TenantId.eq(tid)),
        None => query.filter(i18n_translations::Column::TenantId.is_null()),
    };

    query.filter(build_pair_filter(pairs)).all(db).await
}

pub async fn fetch_global_rows(
    db: &DatabaseConnection,
    pairs: &[(String, String)],
) -> Result<Vec<i18n_translations::Model>, DbErr> {
    if pairs.is_empty() {
        return Ok(Vec::new());
    }

    i18n_translations::Entity::find()
        .filter(i18n_translations::Column::TenantId.is_null())
        .filter(build_pair_filter(pairs))
        .all(db)
        .await
}

pub async fn fetch_tenant_rows(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    pairs: &[(String, String)],
) -> Result<Vec<i18n_translations::Model>, DbErr> {
    if pairs.is_empty() {
        return Ok(Vec::new());
    }

    i18n_translations::Entity::find()
        .filter(i18n_translations::Column::TenantId.eq(tenant_id))
        .filter(build_pair_filter(pairs))
        .all(db)
        .await
}

fn build_distinct_keys_where(
    scope: &KeyScope,
    namespace: Option<&str>,
    q: Option<&str>,
    empty_locale: Option<&str>,
) -> (String, Vec<sea_orm::Value>) {
    let mut binds: Vec<sea_orm::Value> = Vec::new();
    let mut where_sql = match scope {
        KeyScope::GlobalOnly => String::from("tenant_id IS NULL"),
        KeyScope::TenantOnly(tid) => {
            binds.push((*tid).into());
            String::from("tenant_id = $1")
        }
        KeyScope::TenantWithGlobal(tid) => {
            binds.push((*tid).into());
            String::from("(tenant_id IS NULL OR tenant_id = $1)")
        }
    };

    if let Some(ns) = namespace {
        binds.push(ns.to_string().into());
        where_sql.push_str(&format!(" AND namespace = ${}", binds.len()));
    }
    if let Some(needle) = q {
        let pat = format!("%{needle}%");
        binds.push(pat.clone().into());
        let key_idx = binds.len();
        binds.push(pat.into());
        let val_idx = binds.len();
        where_sql.push_str(&format!(
            " AND (key LIKE ${key_idx} OR value LIKE ${val_idx})"
        ));
    }

    if let Some(locale) = empty_locale {
        // For TenantWithGlobal, check that NEITHER a global nor a tenant row
        // exists with a non-empty value for this (namespace, key, locale).
        let scope_filter = match scope {
            KeyScope::GlobalOnly => "sub.tenant_id IS NULL".to_string(),
            KeyScope::TenantOnly(tid) => {
                binds.push((*tid).into());
                format!("sub.tenant_id = ${}", binds.len())
            }
            KeyScope::TenantWithGlobal(tid) => {
                binds.push((*tid).into());
                format!(
                    "(sub.tenant_id IS NULL OR sub.tenant_id = ${})",
                    binds.len()
                )
            }
        };
        binds.push(locale.to_string().into());
        let loc_idx = binds.len();
        where_sql.push_str(&format!(
            " AND NOT EXISTS (\
             SELECT 1 FROM i18n_translations sub \
             WHERE sub.namespace = i18n_translations.namespace \
             AND sub.key = i18n_translations.key \
             AND {scope_filter} \
             AND sub.locale = ${loc_idx} \
             AND sub.value != '')"
        ));
    }

    (where_sql, binds)
}

fn build_pair_filter(pairs: &[(String, String)]) -> Condition {
    let mut pair_filter = Condition::any();
    for (namespace, key) in pairs {
        pair_filter = pair_filter.add(
            Condition::all()
                .add(i18n_translations::Column::Namespace.eq(namespace.clone()))
                .add(i18n_translations::Column::Key.eq(key.clone())),
        );
    }
    pair_filter
}
