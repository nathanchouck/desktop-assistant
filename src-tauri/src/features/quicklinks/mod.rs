//! Quicklinks: a keyword plus text opens a web search or link (`yt cats`,
//! `gh tauri`). The keyword alone offers the link and fills in "yt " so you
//! can type the rest.

mod links;

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use super::ProviderContext;
use crate::search::{glyph, Action, Activation, ItemKind, ResultItem, SearchError, SearchProvider};
use crate::settings::SharedSettings;
use links::Match;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct QuicklinkSettings {
    pub enabled: bool,
    pub links: Vec<Quicklink>,
}

/// A web search or link opened by typing its keyword.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct Quicklink {
    /// Typed first: "yt".
    pub keyword: String,
    pub name: String,
    /// An http(s) address; `{query}` is replaced by the text after the
    /// keyword. Without it the link just opens.
    pub url: String,
}

impl Default for QuicklinkSettings {
    fn default() -> Self {
        let link = |keyword: &str, name: &str, url: &str| Quicklink {
            keyword: keyword.into(),
            name: name.into(),
            url: url.into(),
        };
        Self {
            enabled: true,
            links: vec![
                link("g", "Google", "https://www.google.com/search?q={query}"),
                link(
                    "yt",
                    "YouTube",
                    "https://www.youtube.com/results?search_query={query}",
                ),
                link("gh", "GitHub", "https://github.com/search?q={query}"),
                link("maps", "Google Maps", "https://www.google.com/maps/search/{query}"),
                link(
                    "wiki",
                    "Wikipedia",
                    "https://en.wikipedia.org/w/index.php?search={query}",
                ),
                link("amazon", "Amazon", "https://www.amazon.com/s?k={query}"),
                link(
                    "translate",
                    "Google Translate",
                    "https://translate.google.com/?sl=auto&tl=en&op=translate&text={query}",
                ),
                link("ddg", "DuckDuckGo", "https://duckduckgo.com/?q={query}"),
            ],
        }
    }
}

const ID_PREFIX: &str = "link:";
/// "yt lofi": above apps and files, since you typed a keyword on purpose.
const SEARCH_SCORE: f32 = 1.5;
/// A link without `{query}` whose keyword you typed.
const LINK_SCORE: f32 = 1.2;
/// "yt" alone: below a good app match ("g" is also how Git Bash starts).
const KEYWORD_SCORE: f32 = 0.5;
/// The start of a link's name ("youtu").
const NAME_SCORE: f32 = 0.3;

/// This feature's search results.
pub fn provider(ctx: &ProviderContext) -> Option<Arc<dyn SearchProvider>> {
    Some(Arc::new(QuicklinkProvider {
        settings: Arc::clone(&ctx.settings),
    }))
}

/// Nothing runs in the background.
pub fn start(_app: &AppHandle) {}

pub struct QuicklinkProvider {
    settings: SharedSettings,
}

impl QuicklinkProvider {
    fn current(&self) -> QuicklinkSettings {
        match self.settings.read() {
            Ok(s) => s.quicklinks.clone(),
            Err(poisoned) => poisoned.into_inner().quicklinks.clone(),
        }
    }
}

fn item(found: &Match) -> ResultItem {
    match *found {
        Match::Search(link, text) => ResultItem::new(
            format!("{ID_PREFIX}{}:{text}", link.keyword.trim().to_lowercase()),
            ItemKind::Link,
            format!("Search {} for “{text}”", link.name),
        )
        .with_subtitle(links::host(&link.url))
        .with_icon(glyph("search"))
        .with_accessory(link.keyword.trim())
        .with_score(SEARCH_SCORE)
        .with_actions([Action::Open]),
        Match::Link(link, exact) => {
            let searches = links::takes_text(link);
            let (title, subtitle, icon) = if searches {
                (
                    format!("Search {}", link.name),
                    "Type to search".to_owned(),
                    "search",
                )
            } else {
                (link.name.clone(), links::host(&link.url).to_owned(), "link")
            };
            let score = match (exact, searches) {
                (false, _) => NAME_SCORE,
                (true, true) => KEYWORD_SCORE,
                (true, false) => LINK_SCORE,
            };
            ResultItem::new(
                format!("{ID_PREFIX}{}", link.keyword.trim().to_lowercase()),
                ItemKind::Link,
                title,
            )
            .with_subtitle(subtitle)
            .with_icon(glyph(icon))
            .with_accessory(link.keyword.trim())
            .with_score(score)
            .with_actions([Action::Open])
        }
    }
}

#[async_trait]
impl SearchProvider for QuicklinkProvider {
    fn name(&self) -> &'static str {
        "quicklinks"
    }

    async fn query(&self, query: &str, limit: usize) -> Result<Vec<ResultItem>, SearchError> {
        let settings = self.current();
        if !settings.enabled {
            return Ok(Vec::new());
        }
        Ok(links::find(&settings.links, query)
            .iter()
            .take(limit)
            .map(item)
            .collect())
    }

    fn owns(&self, id: &str) -> bool {
        id.starts_with(ID_PREFIX)
    }

    async fn activate(&self, id: &str, _action: Action) -> Result<Activation, SearchError> {
        let rest = id.strip_prefix(ID_PREFIX).unwrap_or(id);
        let (keyword, text) = match rest.split_once(':') {
            Some((keyword, text)) => (keyword, Some(text)),
            None => (rest, None),
        };
        let settings = self.current();
        let Some(link) = settings
            .links
            .iter()
            .find(|l| links::is_usable(l) && l.keyword.trim().eq_ignore_ascii_case(keyword))
        else {
            return Err(SearchError::Provider {
                provider: self.name(),
                message: format!("there's no quicklink “{keyword}” any more"),
            });
        };
        Ok(match text {
            Some(text) => Activation::OpenUrl {
                url: links::build_url(link, text),
            },
            // The keyword alone: fill it in so you can type what to search.
            None if links::takes_text(link) => Activation::SetQuery {
                query: format!("{} ", link.keyword.trim()),
            },
            None => Activation::OpenUrl {
                url: links::build_url(link, ""),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;
    use std::sync::RwLock;

    fn provider(settings: Settings) -> QuicklinkProvider {
        QuicklinkProvider {
            settings: Arc::new(RwLock::new(settings)),
        }
    }

    #[test]
    fn defaults_are_usable_web_searches() {
        let defaults = QuicklinkSettings::default();
        let keywords: Vec<&str> = defaults.links.iter().map(|l| l.keyword.as_str()).collect();
        assert_eq!(
            keywords,
            ["g", "yt", "gh", "maps", "wiki", "amazon", "translate", "ddg"]
        );
        assert!(defaults
            .links
            .iter()
            .all(|l| links::is_usable(l) && links::takes_text(l)));
    }

    #[tokio::test]
    async fn search_row_opens_the_search() {
        let p = provider(Settings::default());
        let items = p.query("yt lofi beats", 32).await.unwrap();
        assert_eq!(items.len(), 1);
        let row = &items[0];
        assert_eq!(row.title, "Search YouTube for “lofi beats”");
        assert_eq!(row.subtitle, "www.youtube.com");
        assert_eq!(row.kind, ItemKind::Link);
        assert_eq!(row.score, SEARCH_SCORE);
        assert!(p.owns(&row.id));
        assert_eq!(
            p.activate(&row.id, Action::Open).await.unwrap(),
            Activation::OpenUrl {
                url: "https://www.youtube.com/results?search_query=lofi%20beats".into()
            }
        );
        // Text with a colon survives the id.
        let items = p.query("g 10:30 am in tokyo", 32).await.unwrap();
        assert_eq!(
            p.activate(&items[0].id, Action::Open).await.unwrap(),
            Activation::OpenUrl {
                url: "https://www.google.com/search?q=10%3A30%20am%20in%20tokyo".into()
            }
        );
    }

    #[tokio::test]
    async fn keyword_alone_fills_in_the_query() {
        let p = provider(Settings::default());
        let items = p.query("yt", 32).await.unwrap();
        assert_eq!(items[0].title, "Search YouTube");
        assert_eq!(items[0].subtitle, "Type to search");
        assert_eq!(items[0].score, KEYWORD_SCORE);
        assert_eq!(
            p.activate(&items[0].id, Action::Open).await.unwrap(),
            Activation::SetQuery { query: "yt ".into() }
        );
    }

    #[tokio::test]
    async fn plain_links_just_open() {
        let mut settings = Settings::default();
        settings.quicklinks.links.push(Quicklink {
            keyword: "Mail".into(),
            name: "Gmail".into(),
            url: "https://mail.google.com/".into(),
        });
        let p = provider(settings);
        let items = p.query("mail", 32).await.unwrap();
        assert_eq!(items[0].title, "Gmail");
        assert_eq!(items[0].score, LINK_SCORE);
        assert_eq!(
            p.activate(&items[0].id, Action::Open).await.unwrap(),
            Activation::OpenUrl {
                url: "https://mail.google.com/".into()
            }
        );
    }

    #[tokio::test]
    async fn off_or_removed() {
        let mut settings = Settings::default();
        settings.quicklinks.enabled = false;
        let p = provider(settings);
        assert!(p.query("yt cats", 32).await.unwrap().is_empty());
        let p = provider(Settings::default());
        assert!(p.activate("link:nope:x", Action::Open).await.is_err());
        assert!(p.query("chrome", 32).await.unwrap().is_empty());
    }
}
