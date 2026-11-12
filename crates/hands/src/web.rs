//! The web, with sources: `search` gives the top results (title, link, snippet) from DuckDuckGo's plain HTML page
//! (no key, nothing signed in), `read` gives a page as text. What she says from it, she says with where it came from.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::tool::{Ctx, Outcome, Risk, Tool, ToolSpec, arg, opt_arg};

/// One search result.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// The results on DuckDuckGo's HTML page, in order.
pub fn hits(html: &str) -> Vec<Hit> {
    let mut out = Vec::new();
    for block in html.split("class=\"result__a\"").skip(1) {
        let Some(href) = attr(block, "href=\"") else { continue };
        // Links go through DuckDuckGo's redirect: the real one is its `uddg` parameter.
        let url = href.split("uddg=").nth(1).map(|u| decode_url(u.split('&').next().unwrap_or(u))).unwrap_or_else(|| href.to_string());
        if !url.starts_with("http") || url.contains("duckduckgo.com/y.js") {
            continue; // an ad
        }
        let title = block.split_once('>').map(|(_, rest)| text(rest.split("</a>").next().unwrap_or(""))).unwrap_or_default();
        let snippet = block
            .split("class=\"result__snippet\"")
            .nth(1)
            .and_then(|s| s.split_once('>'))
            .map(|(_, rest)| text(rest.split("</a>").next().unwrap_or("")))
            .unwrap_or_default();
        if !title.is_empty() && !out.iter().any(|h: &Hit| h.url == url) {
            out.push(Hit { title, url, snippet });
        }
    }
    out
}

/// The results on Bing's page (the fallback when DuckDuckGo turns automated searches away). Its links go through
/// a redirect whose `u=a1…` parameter is the real one, base64.
pub fn bing_hits(html: &str) -> Vec<Hit> {
    use base64::Engine as _;
    let mut out = Vec::new();
    for block in html.split("<li class=\"b_algo\"").skip(1) {
        let block = block.split("</li>").next().unwrap_or(block);
        let Some(h2) = block.split("<h2").nth(1) else { continue };
        let Some(href) = attr(h2, "href=\"") else { continue };
        let href = href.replace("&amp;", "&");
        let url = match href.split("&u=a1").nth(1).map(|u| u.split('&').next().unwrap_or(u)) {
            Some(b64) => base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(b64.trim_end_matches('=')).ok().and_then(|b| String::from_utf8(b).ok()).unwrap_or_default(),
            None => href.clone(),
        };
        let title = h2.split_once("\">").map(|(_, r)| text(r.split("</a>").next().unwrap_or(""))).unwrap_or_default();
        let snippet = block.split("<p").nth(1).and_then(|p| p.split_once('>')).map(|(_, r)| text(r.split("</p>").next().unwrap_or(""))).unwrap_or_default();
        if url.starts_with("http") && !title.is_empty() && !out.iter().any(|h: &Hit| h.url == url) {
            out.push(Hit { title, url, snippet });
        }
    }
    out
}

/// The results on Brave Search's page: the fallback when DuckDuckGo turns a quick second search away with an empty
/// page (Bing's page came back without results to read, and she said there were none).
pub fn brave_hits(html: &str) -> Vec<Hit> {
    let mut out = Vec::new();
    for block in html.split("data-type=\"web\"").skip(1) {
        let block = &block[..block.len().min(8000)];
        let Some(url) = attr(block, "<a href=\"").map(|u| u.replace("&amp;", "&")) else { continue };
        let title = block.split("search-snippet-title").nth(1).and_then(|t| attr(t, "title=\"")).map(text).unwrap_or_default();
        let snippet = block.split("<div class=\"content").nth(1).and_then(|c| c.split_once('>')).map(|(_, r)| text(r.split("</div>").next().unwrap_or(""))).unwrap_or_default();
        if url.starts_with("http") && !url.contains("brave.com") && !title.is_empty() && !out.iter().any(|h: &Hit| h.url == url) {
            out.push(Hit { title, url, snippet });
        }
    }
    out
}

fn attr<'a>(s: &'a str, start: &str) -> Option<&'a str> {
    let i = s.find(start)? + start.len();
    s[i..].split('"').next()
}

/// `%2F` → `/`, `+` → space.
fn decode_url(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => match std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                Some(b) => {
                    out.push(b);
                    i += 3;
                    continue;
                }
                None => out.push(b'%'),
            },
            b'+' => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// HTML to plain text: scripts, styles and tags out, entities read, whitespace collapsed.
pub fn text(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let lower = html.to_ascii_lowercase();
    let mut i = 0;
    while i < html.len() {
        if html[i..].starts_with('<') {
            // Skip whole <script> and <style> elements, then any tag.
            let skip_to = ["script", "style", "noscript", "svg"].iter().find_map(|t| lower[i + 1..].starts_with(t).then(|| lower[i..].find(&format!("</{t}>")).map(|e| i + e + t.len() + 3)));
            i = match skip_to {
                Some(Some(end)) => end,
                Some(None) => html.len(),
                None => html[i..].find('>').map_or(html.len(), |e| i + e + 1),
            };
            out.push(' ');
            continue;
        }
        let c = html[i..].chars().next().expect("in bounds");
        out.push(c);
        i += c.len_utf8();
    }
    let out = out.replace("&amp;", "&").replace("&quot;", "\"").replace("&#x27;", "'").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&nbsp;", " ");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Wikipedia's best article for `query`: its title, link and plain text (the start of it), plus the next titles.
async fn wiki(query: &str) -> Outcome {
    const API: &str = "https://en.wikipedia.org/w/api.php";
    let http = reqwest::Client::builder().timeout(Duration::from_secs(10)).user_agent("Capricorn/0.1 (github.com/theluckytiger/capricorn-engram)").build().expect("http client");
    let found: Value = match http.get(API).query(&[("action", "query"), ("list", "search"), ("srsearch", query), ("srlimit", "4"), ("format", "json")]).send().await {
        Ok(r) => r.json().await.unwrap_or_default(),
        Err(e) => return Outcome::failed(format!("couldn't reach Wikipedia: {e}")),
    };
    let titles: Vec<String> = found["query"]["search"].as_array().into_iter().flatten().filter_map(|t| t["title"].as_str().map(String::from)).collect();
    let Some(title) = titles.first() else { return Outcome::failed(format!("Wikipedia has nothing for \"{query}\"")) };
    let page: Value = match http.get(API).query(&[("action", "query"), ("prop", "extracts"), ("explaintext", "1"), ("redirects", "1"), ("titles", title.as_str()), ("format", "json")]).send().await {
        Ok(r) => r.json().await.unwrap_or_default(),
        Err(e) => return Outcome::failed(format!("couldn't read Wikipedia: {e}")),
    };
    let body = page["query"]["pages"].as_object().and_then(|p| p.values().next()).and_then(|p| p["extract"].as_str()).unwrap_or_default();
    let cut: String = body.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(PAGE_CHARS).collect();
    let url = format!("https://en.wikipedia.org/wiki/{}", title.replace(' ', "_"));
    let others = titles.iter().skip(1).cloned().collect::<Vec<_>>().join("; ");
    Outcome::ok(format!("Wikipedia, \"{title}\" ({url}): {cut}\n(Other articles: {others})")).with(json!({ "links": [url], "titles": titles }))
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().timeout(Duration::from_secs(10)).user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0 Safari/537.36").build().expect("http client")
}

/// How much of a page goes to the model: enough for an article's facts, few enough tokens to stay quick.
const PAGE_CHARS: usize = 6000;

pub struct Web;

#[async_trait]
impl Tool for Web {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "web".into(),
            description: "Look things up on the web. `wiki` (with `query`): Wikipedia's article on it, for facts about \
                          people, places, events, champions, records, who holds an office (search it with the year when \
                          it changes year to year). `search` (with `query`): the top results with their links and a \
                          snippet, for news, prices, releases, anything recent. `read` (with `url`): a page's text, when \
                          a snippet doesn't answer it. Say which site it came from."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["wiki", "search", "read"] },
                    "query": { "type": "string" },
                    "url": { "type": "string" }
                },
                "required": ["action"]
            }),
        }
    }

    fn risk(&self, _: &Value) -> Risk {
        Risk::Read
    }

    async fn call(&self, args: Value, _ctx: &Ctx) -> Outcome {
        let action: String = match arg(&args, "action") {
            Ok(a) => a,
            Err(o) => return o,
        };
        match action.as_str() {
            "search" => {
                let Some(query) = opt_arg::<String>(&args, "query").filter(|q| !q.trim().is_empty()) else { return Outcome::failed("search needs a query") };
                // English results wherever the PC is: outside English-speaking countries, "new F1 world champion" came
                // back as local news homepages.
                let html = match client().get("https://html.duckduckgo.com/html/").query(&[("q", query.as_str()), ("kl", "us-en")]).send().await {
                    Ok(r) => r.text().await.unwrap_or_default(),
                    Err(e) => return Outcome::failed(format!("couldn't reach the search: {e}")),
                };
                let mut found = hits(&html);
                if found.is_empty() {
                    // DuckDuckGo turns away a burst of automated searches with an empty page: ask Brave, then Bing.
                    let page = client().get("https://search.brave.com/search").query(&[("q", query.as_str())]).header("Accept-Language", "en-US,en;q=0.9").send().await;
                    if let Ok(r) = page {
                        found = brave_hits(&r.text().await.unwrap_or_default());
                    }
                }
                if found.is_empty() {
                    let page = client().get("https://www.bing.com/search").query(&[("q", query.as_str()), ("setlang", "en"), ("cc", "US"), ("mkt", "en-US")]).header("Accept-Language", "en-US,en;q=0.9").send().await;
                    if let Ok(r) = page {
                        found = bing_hits(&r.text().await.unwrap_or_default());
                    }
                }
                if found.is_empty() {
                    return Outcome::failed(format!("no results for \"{query}\""));
                }
                let list: Vec<String> = found.iter().take(6).enumerate().map(|(i, h)| format!("{}. {} ({}): {}", i + 1, h.title, h.url, h.snippet)).collect();
                // Snippets are a line each: an answer that isn't in them is on the pages, not missing. Searching again
                // in other words doesn't help (it went four rounds once); reading does.
                Outcome::ok(format!("results for \"{query}\" (snippets only: if the answer isn't in them, don't search again; web read the likeliest link):\n{}", list.join("\n")))
                    .with(json!({ "links": found.iter().take(6).map(|h| h.url.clone()).collect::<Vec<_>>() }))
            }
            "wiki" => {
                let Some(query) = opt_arg::<String>(&args, "query").filter(|q| !q.trim().is_empty()) else { return Outcome::failed("wiki needs a query") };
                wiki(&query).await
            }
            "read" => {
                let Some(url) = opt_arg::<String>(&args, "url").filter(|u| u.starts_with("http")) else { return Outcome::failed("read needs a url (http…)") };
                let html = match client().get(&url).header("Accept-Language", "en-US,en;q=0.9").header("Accept", "text/html,application/xhtml+xml,*/*;q=0.8").send().await {
                    Ok(r) if r.status().is_success() => r.text().await.unwrap_or_default(),
                    // Shops behind a bot wall (gg.deals, loaded.com) turn her plain reader away: their page in the
                    // browser shows it all the same.
                    Ok(r) if matches!(r.status().as_u16(), 401 | 403 | 429 | 503) => {
                        return Outcome::failed(format!("{url} answered {} (it turns away anything but a browser): open it in the browser with open, then read_window or look at it", r.status()));
                    }
                    Ok(r) => return Outcome::failed(format!("{url} answered {}", r.status())),
                    Err(e) => return Outcome::failed(format!("couldn't open {url}: {e}")),
                };
                let page = text(&html);
                if page.is_empty() {
                    return Outcome::failed(format!("{url} has no readable text (it may need a browser)"));
                }
                let cut: String = page.chars().take(PAGE_CHARS).collect();
                Outcome::ok(format!("{url} says: {cut}{}", if page.chars().count() > PAGE_CHARS { " …" } else { "" }))
            }
            other => Outcome::failed(format!("unknown action \"{other}\"")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_results() {
        let html = r#"<a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fvideocardprices.com%2Fcard%2Fnvidia%2Drtx%2D5080%2Dsuper%2F&amp;rut=4c">RTX 5080 Super Price Tracker &amp; more</a>
            <a class="result__snippet" href="x">Track <b>RTX</b> 5080 Super prices</a>
            <a class="result__a" href="//duckduckgo.com/y.js?ad=1">An ad</a>"#;
        let h = hits(html);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].url, "https://videocardprices.com/card/nvidia-rtx-5080-super/");
        assert_eq!(h[0].title, "RTX 5080 Super Price Tracker & more");
        assert_eq!(h[0].snippet, "Track RTX 5080 Super prices");
    }

    #[test]
    fn reads_brave() {
        let html = r#"<div class="snippet svelte-jmfu5f" data-pos="0" data-type="web"><div class="result-content"><a href="https://www.loaded.com/gta-5-pc" target="_self" class="l1"><cite>loaded.com</cite><div class="title search-snippet-title line-clamp-1" title="Buy GTA V PC CD Key | Loaded">Buy GTA V PC CD Key | Loaded</div></a><div class="content desktop-default-regular">Instant delivery and the <strong>best</strong> prices</div></div></div><div data-type="web"><a href="https://search.brave.com/x"><div class="search-snippet-title" title="Brave">x</div></a></div>"#;
        let h = super::brave_hits(html);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].url, "https://www.loaded.com/gta-5-pc");
        assert_eq!(h[0].title, "Buy GTA V PC CD Key | Loaded");
        assert_eq!(h[0].snippet, "Instant delivery and the best prices");
    }

    #[test]
    fn reads_bing() {
        let html = r#"<li class="b_algo" data-id><h2 class=""><a target="_blank" href="https://www.bing.com/ck/a?!&amp;&amp;p=44c5&amp;u=a1aHR0cHM6Ly93d3cucmVhbG1hZHJpZC5jb20vZW4&amp;ntb=1">Real Madrid CF | Official Website</a></h2><div class="b_caption"><p class="b_lineclamp2">Welcome to the <strong>official</strong> website.</p></div></li>"#;
        let h = bing_hits(html);
        assert_eq!(h, vec![Hit { title: "Real Madrid CF | Official Website".into(), url: "https://www.realmadrid.com/en".into(), snippet: "Welcome to the official website.".into() }]);
    }

    #[test]
    fn pages_as_text() {
        assert_eq!(text("<html><script>var x=1;</script><h1>Hi</h1><p>there &amp; back</p><style>p{}</style></html>"), "Hi there & back");
    }
}
