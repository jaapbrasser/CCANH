//! Shared pagination parameters (SPEC §13.1): `?page=&page_size=`, max 200.

use super::error::ApiError;

pub const DEFAULT_PAGE_SIZE: u32 = 50;
pub const MAX_PAGE_SIZE: u32 = 200;

/// Raw `page` / `page_size` query values. Query structs declare these two fields
/// themselves (serde `flatten` breaks number parsing in query strings).
#[derive(Debug, Clone, Copy, Default)]
pub struct PageParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}

/// Validated pagination.
#[derive(Debug, Clone, Copy)]
pub struct Page {
    pub page: u32,
    pub page_size: u32,
}

impl Page {
    pub fn limit(self) -> i64 {
        i64::from(self.page_size)
    }

    pub fn offset(self) -> i64 {
        i64::from(self.page - 1) * i64::from(self.page_size)
    }
}

impl TryFrom<PageParams> for Page {
    type Error = ApiError;

    fn try_from(p: PageParams) -> Result<Self, ApiError> {
        let page = p.page.unwrap_or(1);
        let page_size = p.page_size.unwrap_or(DEFAULT_PAGE_SIZE);
        if page == 0 {
            return Err(ApiError::BadRequest("page must be at least 1".into()));
        }
        if page_size == 0 || page_size > MAX_PAGE_SIZE {
            return Err(ApiError::BadRequest(format!(
                "page_size must be between 1 and {MAX_PAGE_SIZE}"
            )));
        }
        Ok(Page { page, page_size })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_bounds() {
        let p = Page::try_from(PageParams::default()).unwrap();
        assert_eq!((p.page, p.page_size, p.offset()), (1, 50, 0));
        let p = Page::try_from(PageParams {
            page: Some(3),
            page_size: Some(20),
        })
        .unwrap();
        assert_eq!(p.offset(), 40);
        assert!(Page::try_from(PageParams {
            page: Some(0),
            page_size: None
        })
        .is_err());
        assert!(Page::try_from(PageParams {
            page: None,
            page_size: Some(201)
        })
        .is_err());
    }
}
