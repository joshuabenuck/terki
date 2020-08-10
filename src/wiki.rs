use anyhow::{anyhow, Error, Result};
use reqwest;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use url::Url;

#[derive(Debug)]
pub enum PageStore {
    Local {
        path: PathBuf,
    },
    Http {
        url: String,
        cache: HashMap<String, String>,
    },
}

impl PageStore {
    async fn retrieve(&mut self, slug: &str) -> Result<Page> {
        let page = match self {
            PageStore::Local { path } => {
                serde_json::from_str(&fs::read_to_string(path.join("pages").join(slug))?)?
            }
            PageStore::Http { url, cache } => {
                if !cache.contains_key(slug) {
                    let url = Url::parse(url)?;
                    let page_url = url.join(&format!("{}.json", slug))?;
                    let body = reqwest::get(page_url).await?.text().await?;
                    cache.insert(slug.to_owned(), body);
                }
                serde_json::from_str(cache.get(slug).as_ref().unwrap())?
            }
        };
        Ok(page)
    }
}
#[derive(Debug)]
pub struct Wiki {
    pub store: PageStore,
    pages: HashMap<String, Page>,
}

impl Wiki {
    pub fn new(store: PageStore) -> Wiki {
        Wiki {
            store,
            pages: HashMap::new(),
        }
    }

    pub async fn page<'a>(&'a mut self, slug: &str) -> Result<&'a mut Page, Error> {
        if !self.pages.contains_key(slug) {
            let retrieved = self.store.retrieve(&slug).await?;
            self.pages.insert(slug.to_owned(), retrieved);
        }
        Ok(self.pages.get_mut(slug).unwrap())
    }
}

#[derive(Deserialize, Debug)]
struct Item {
    id: String,
    r#type: String,
    text: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(tag = "type")]
enum JournalEntry {
    Create {
        item: Item,
        date: u64,
    },
    Add {
        id: String,
        after: String,
        date: u64,
        item: Item,
    },
    Edit {
        id: String,
        item: Item,
        date: u64,
    },
    Remove {
        id: String,
        date: u64,
    },
    Move {
        id: String,
        order: Vec<String>,
    },
    Fork {
        data: u64,
    },
}

#[derive(Deserialize, Debug)]
pub struct Page {
    title: String,
    story: Vec<Item>,
    journal: Value,
    #[serde(skip)]
    links: Vec<(String, String)>,
    #[serde(skip)]
    // the item a line belongs to
    line_item: Vec<Option<usize>>,
}

impl Page {
    fn render_item(&self, cols: usize, item: &Item) -> Vec<String> {
        let mut lines = Vec::new();
        let mut prefix = "";
        if item.r#type == "pagefold" {
            let heading = format!(" {} ", item.text.as_deref().unwrap_or(""));
            lines.push(format!("{:-^1$}", heading, cols));
            return lines;
        }
        if item.r#type != "paragraph" {
            prefix = "  ";
            lines.push(item.r#type.to_owned());
        }
        let text = item.text.as_deref().unwrap_or("<empty>");
        if item.r#type == "paragraph" {
            // search for links
            // for each link
            // add to links
            // render shortened external link
            // render as a link
        }
        for line in text.split("\n") {
            for l in textwrap::wrap_iter(&line, cols - prefix.len()) {
                lines.push(format!("{}{}", prefix, l.to_string()));
            }
        }
        return lines;
    }

    pub fn lines(&mut self, cols: usize) -> Vec<String> {
        let mut lines = Vec::new();
        for (i, item) in self.story.iter().enumerate() {
            for line in self.render_item(cols, item) {
                self.line_item.push(Some(i));
                lines.push(line);
            }
            self.line_item.push(None);
            lines.push("".to_string());
        }
        lines
    }
}
