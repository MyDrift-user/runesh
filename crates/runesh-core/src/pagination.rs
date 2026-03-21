//! Pagination query extractor and response wrapper.
//!
//! Pairs with `@runesh/ui`'s `PaginatedResponse<T>` and `DataTable` component.

use serde::{Deserialize, Serialize};

/// Pagination query parameters. Use as an Axum query extractor:
///
/// ```ignore
/// async fn list(Query(pg): Query<Pagination>) -> Result<Json<PaginatedResponse<Item>>, AppError> {
///     let items = sqlx::query_as("SELECT * FROM items ORDER BY $1 LIMIT $2 OFFSET $3")
///         .bind(pg.sort_by.as_deref().unwrap_or("created_at"))
///         .bind(pg.limit())
///         .bind(pg.offset())
///         .fetch_all(&pool).await?;
///     let total = sqlx::query_scalar("SELECT COUNT(*) FROM items")
///         .fetch_one(&pool).await?;
///     Ok(Json(pg.response(items, total)))
/// }
/// ```
#[derive(Debug, Deserialize)]
pub struct Pagination {
    #[serde(default = "default_page")]
    pub page: i64,
    #[serde(default = "default_page_size")]
    pub page_size: i64,
    pub sort_by: Option<String>,
    #[serde(default)]
    pub sort_dir: Option<String>,
    pub search: Option<String>,
}

fn default_page() -> i64 { 0 }
fn default_page_size() -> i64 { 25 }

impl Pagination {
    pub fn limit(&self) -> i64 {
        self.page_size.clamp(1, 100)
    }

    pub fn offset(&self) -> i64 {
        self.page.max(0) * self.limit()
    }

    pub fn sort_direction(&self) -> &str {
        match self.sort_dir.as_deref() {
            Some("desc" | "DESC") => "DESC",
            _ => "ASC",
        }
    }

    /// Build a `PaginatedResponse` from items and total count.
    pub fn response<T: Serialize>(&self, items: Vec<T>, total: i64) -> PaginatedResponse<T> {
        PaginatedResponse {
            items,
            total,
            page: self.page,
            page_size: self.limit(),
        }
    }
}

/// Paginated API response. Matches the frontend `PaginatedResponse<T>` type.
#[derive(Debug, Serialize)]
pub struct PaginatedResponse<T: Serialize> {
    pub items: Vec<T>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}
