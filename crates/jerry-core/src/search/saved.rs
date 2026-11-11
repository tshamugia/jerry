//! A search as the user puts it in Jerry's search window (#66): what the search history keeps
//! (`search_history`, the last 20) and a results tab runs again. Unlike [`SearchQuery`], its dates
//! can be relative ("this week"), so a search run again next week looks at that week; the UI turns
//! it into a query in the user's time zone.
//!
//! [`SearchQuery`]: super::SearchQuery

use serde::{Deserialize, Serialize};
use specta::Type;

use super::{FileType, SearchItems, SizeRange};

/// A search: its name text and filters, and whether it takes the whole drive.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SavedSearch {
    /// The name text as typed: words, `*` and `?` (`SearchQuery::name`).
    pub name: String,
    pub file_type: Option<FileType>,
    pub size: Option<SizeRange>,
    pub modified: Option<DateFilter>,
    pub items: SearchItems,
    /// The whole drive the folder is on, from its root, not only the folder and what is in it.
    pub whole_drive: bool,
}

/// When an item was last changed.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DateFilter {
    Today,
    /// Since Monday.
    ThisWeek,
    ThisMonth,
    /// Days the user chose, as `DateRange` takes them: ms since the Unix epoch, `from` included,
    /// `to` left out. A missing end is open.
    Range {
        from: Option<f64>,
        to: Option<f64>,
    },
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn saved_searches_read_back_as_written() {
        let search = SavedSearch {
            name: "ნახაზი *.dwg".to_owned(),
            file_type: Some(FileType::Drawings),
            size: Some(SizeRange {
                min: Some(1024.0),
                max: None,
            }),
            modified: Some(DateFilter::ThisWeek),
            items: SearchItems::Files,
            whole_drive: true,
        };
        let json = serde_json::to_value(&search).expect("serializes");
        assert_eq!(
            json,
            json!({
                "name": "ნახაზი *.dwg",
                "fileType": { "kind": "drawings" },
                "size": { "min": 1024.0, "max": null },
                "modified": { "kind": "thisWeek" },
                "items": "files",
                "wholeDrive": true,
            })
        );
        assert_eq!(
            serde_json::from_value::<SavedSearch>(json).ok(),
            Some(search)
        );
        let range = json!({ "kind": "range", "from": 1.0, "to": null });
        assert_eq!(
            serde_json::from_value::<DateFilter>(range).ok(),
            Some(DateFilter::Range {
                from: Some(1.0),
                to: None
            })
        );
    }
}
