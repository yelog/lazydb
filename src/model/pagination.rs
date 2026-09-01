use std::{convert::TryFrom, ops::RangeInclusive};

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum PageSize {
    Ten,
    OneHundred,
    TwoHundredFifty,
    #[default]
    FiveHundred,
    OneThousand,
}

impl PageSize {
    pub const ALL: [Self; 5] = [
        Self::Ten,
        Self::OneHundred,
        Self::TwoHundredFifty,
        Self::FiveHundred,
        Self::OneThousand,
    ];

    pub const fn get(self) -> usize {
        match self {
            Self::Ten => 10,
            Self::OneHundred => 100,
            Self::TwoHundredFifty => 250,
            Self::FiveHundred => 500,
            Self::OneThousand => 1000,
        }
    }

    pub const fn lookahead_limit(self) -> usize {
        self.get() + 1
    }

    pub const fn as_u64(self) -> u64 {
        self.get() as u64
    }
}

impl TryFrom<usize> for PageSize {
    type Error = ();

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        match value {
            10 => Ok(Self::Ten),
            100 => Ok(Self::OneHundred),
            250 => Ok(Self::TwoHundredFifty),
            500 => Ok(Self::FiveHundred),
            1000 => Ok(Self::OneThousand),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PageRequest {
    pub size: PageSize,
    pub offset: u64,
    pub resolve_total: bool,
}

impl PageRequest {
    pub const fn first(size: PageSize) -> Self {
        Self::at(size, 0)
    }

    pub const fn at(size: PageSize, offset: u64) -> Self {
        Self {
            size,
            offset,
            resolve_total: false,
        }
    }

    pub const fn last(size: PageSize, total_hint: u64) -> Self {
        Self {
            size,
            offset: ResultPagination::last_offset(size, total_hint),
            resolve_total: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TotalRows {
    LowerBound(u64),
    Exact(u64),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResultPagination {
    pub page_size: PageSize,
    pub offset: u64,
    pub visible_rows: usize,
    pub has_next: bool,
    pub total: TotalRows,
}

impl ResultPagination {
    pub fn from_page(request: PageRequest, fetched_rows: usize) -> Self {
        let page_size = request.size.get();
        let visible_rows = fetched_rows.min(page_size);
        let has_next = fetched_rows > page_size;
        let total = if has_next {
            TotalRows::LowerBound(
                request
                    .offset
                    .saturating_add(page_size as u64)
                    .saturating_add(1),
            )
        } else {
            TotalRows::Exact(request.offset.saturating_add(visible_rows as u64))
        };
        Self {
            page_size: request.size,
            offset: request.offset,
            visible_rows,
            has_next,
            total,
        }
    }

    pub const fn last_offset(page_size: PageSize, total: u64) -> u64 {
        if total == 0 {
            0
        } else {
            (total - 1) / page_size.as_u64() * page_size.as_u64()
        }
    }

    pub fn range(&self) -> Option<RangeInclusive<u64>> {
        if self.visible_rows == 0 {
            return None;
        }
        let start = self.offset.checked_add(1)?;
        let end = self.offset.checked_add(self.visible_rows as u64)?;
        Some(start..=end)
    }

    pub fn first_request(&self) -> Option<PageRequest> {
        (self.offset > 0).then(|| PageRequest::first(self.page_size))
    }

    pub fn previous_request(&self) -> Option<PageRequest> {
        (self.offset > 0).then(|| {
            PageRequest::at(
                self.page_size,
                self.offset.saturating_sub(self.page_size.as_u64()),
            )
        })
    }

    pub fn next_request(&self) -> Option<PageRequest> {
        self.has_next
            .then(|| self.offset.checked_add(self.page_size.as_u64()))
            .flatten()
            .map(|offset| PageRequest::at(self.page_size, offset))
    }

    pub fn last_request(&self) -> Option<PageRequest> {
        match self.total {
            TotalRows::Exact(total) => {
                let offset = Self::last_offset(self.page_size, total);
                (offset != self.offset).then(|| PageRequest::last(self.page_size, total))
            }
            TotalRows::LowerBound(_) => self.has_next.then(|| PageRequest::last(self.page_size, 0)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PageRequest, PageSize, ResultPagination, TotalRows};

    #[test]
    fn page_sizes_are_closed_and_convertible() {
        assert_eq!(
            PageSize::ALL,
            [
                PageSize::Ten,
                PageSize::OneHundred,
                PageSize::TwoHundredFifty,
                PageSize::FiveHundred,
                PageSize::OneThousand,
            ]
        );
        assert_eq!(PageSize::default().get(), 500);
        assert_eq!(PageSize::OneThousand.lookahead_limit(), 1001);

        for size in PageSize::ALL {
            assert_eq!(PageSize::try_from(size.get()), Ok(size));
        }
        assert!(PageSize::try_from(0).is_err());
        assert!(PageSize::try_from(11).is_err());
        assert!(PageSize::try_from(1001).is_err());
        assert!(PageSize::try_from(usize::MAX).is_err());
    }

    #[test]
    fn pagination_is_derived_from_visible_rows_and_probe() {
        let first = ResultPagination::from_page(PageRequest::first(PageSize::FiveHundred), 501);
        assert_eq!(first.visible_rows, 500);
        assert!(first.has_next);
        assert_eq!(first.total, TotalRows::LowerBound(501));
        assert_eq!(first.range(), Some(1..=500));

        let last = ResultPagination::from_page(PageRequest::at(PageSize::FiveHundred, 1000), 234);
        assert_eq!(last.visible_rows, 234);
        assert!(!last.has_next);
        assert_eq!(last.total, TotalRows::Exact(1234));
        assert_eq!(last.range(), Some(1001..=1234));

        let empty = ResultPagination::from_page(PageRequest::first(PageSize::FiveHundred), 0);
        assert_eq!(empty.total, TotalRows::Exact(0));
        assert_eq!(empty.range(), None);
    }

    #[test]
    fn last_offset_starts_the_page_containing_the_final_row() {
        let size = PageSize::FiveHundred;
        assert_eq!(ResultPagination::last_offset(size, 0), 0);
        assert_eq!(ResultPagination::last_offset(size, 1), 0);
        assert_eq!(ResultPagination::last_offset(size, 500), 0);
        assert_eq!(ResultPagination::last_offset(size, 501), 500);
        assert_eq!(ResultPagination::last_offset(size, 1000), 500);
        assert_eq!(ResultPagination::last_offset(size, 1001), 1000);
    }

    #[test]
    fn navigation_requests_are_safe_and_only_enabled_when_valid() {
        let first = ResultPagination::from_page(PageRequest::first(PageSize::FiveHundred), 501);
        assert_eq!(first.first_request(), None);
        assert_eq!(first.previous_request(), None);
        assert_eq!(
            first.next_request(),
            Some(PageRequest::at(PageSize::FiveHundred, 500))
        );
        assert_eq!(
            first.last_request(),
            Some(PageRequest::last(PageSize::FiveHundred, 0))
        );

        let last = ResultPagination::from_page(PageRequest::at(PageSize::FiveHundred, 1000), 234);
        assert_eq!(
            last.first_request(),
            Some(PageRequest::first(PageSize::FiveHundred))
        );
        assert_eq!(
            last.previous_request(),
            Some(PageRequest::at(PageSize::FiveHundred, 500))
        );
        assert_eq!(last.next_request(), None);
        assert_eq!(last.last_request(), None);

        let middle = ResultPagination::from_page(PageRequest::at(PageSize::FiveHundred, 500), 500);
        assert_eq!(middle.last_request(), None);

        let overflow =
            ResultPagination::from_page(PageRequest::at(PageSize::OneThousand, u64::MAX), 1001);
        assert_eq!(overflow.range(), None);
        assert_eq!(overflow.next_request(), None);
        assert_eq!(overflow.total, TotalRows::LowerBound(u64::MAX));
    }

    #[test]
    fn explicit_last_requests_resolve_the_exact_total() {
        assert_eq!(
            PageRequest::last(PageSize::FiveHundred, 1001),
            PageRequest {
                size: PageSize::FiveHundred,
                offset: 1000,
                resolve_total: true,
            }
        );
    }
}
