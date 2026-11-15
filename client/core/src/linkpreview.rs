//! What a link shows under a message: the title, the description and the picture its page
//! declares. It is fetched by whoever sends the link, so nobody who receives it opens the page,
//! and the page never learns who read the message. Same rules as `desktop/src/linkpreview.js`.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use url::Url;

const TIMEOUT: Duration = Duration::from_secs(8);
const MAX_HTML_BYTES: usize = 1024 * 1024;
const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;
const MAX_URL_LENGTH: usize = 2048;
const CACHE_LIMIT: usize = 60;
const CACHE_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_TITLE: usize = 200;
const MAX_DESCRIPTION: usize = 400;
const MAX_SITE: usize = 80;
const USER_AGENT: &str = "Mozilla/5.0 (compatible; TelinhaBot/1.0; +link preview)";
const IMAGE_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/webp", "image/gif"];

const ENTITIES: [(&str, &str); 52] = [
    ("amp", "&"), ("lt", "<"), ("gt", ">"), ("quot", "\""), ("apos", "'"), ("nbsp", " "), ("hellip", "…"), ("mdash", "—"),
    ("ndash", "–"), ("laquo", "«"), ("raquo", "»"), ("ldquo", "“"), ("rdquo", "”"), ("lsquo", "‘"), ("rsquo", "’"),
    ("middot", "·"), ("bull", "•"), ("copy", "©"), ("reg", "®"), ("trade", "™"), ("deg", "°"), ("ordf", "ª"), ("ordm", "º"),
    ("iexcl", "¡"), ("iquest", "¿"), ("aacute", "á"), ("eacute", "é"), ("iacute", "í"), ("oacute", "ó"), ("uacute", "ú"),
    ("Aacute", "Á"), ("Eacute", "É"), ("Iacute", "Í"), ("Oacute", "Ó"), ("Uacute", "Ú"), ("agrave", "à"), ("Agrave", "À"),
    ("acirc", "â"), ("ecirc", "ê"), ("ocirc", "ô"), ("Acirc", "Â"), ("Ecirc", "Ê"), ("Ocirc", "Ô"), ("atilde", "ã"),
    ("otilde", "õ"), ("Atilde", "Ã"), ("Otilde", "Õ"), ("ccedil", "ç"), ("Ccedil", "Ç"), ("uuml", "ü"), ("ntilde", "ñ"),
    ("Ntilde", "Ñ"),
];

/// Turns `&amp;`, `&#233;` and `&#xe9;` into the letters they stand for. What is not a known
/// name stays as written.
pub fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let decoded = after.find(';').filter(|end| (1..=12).contains(end)).and_then(|end| {
            let code = &after[..end];
            let letter = if let Some(number) = code.strip_prefix('#') {
                let value = match number.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => number.parse().ok(),
                };
                value.filter(|value| *value > 0).and_then(char::from_u32).map(String::from)
            } else {
                let named = |name: &str| ENTITIES.iter().find(|(known, _)| *known == name).map(|(_, letter)| (*letter).to_owned());
                named(code).or_else(|| named(&code.to_lowercase()))
            };
            letter.map(|letter| (letter, end))
        });
        match decoded {
            Some((letter, end)) => {
                out.push_str(&letter);
                rest = &after[end + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn clean_text(value: &str, max: usize) -> String {
    let text = decode_entities(value).split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= max {
        return text;
    }
    let cut: String = text.chars().take(max - 1).collect();
    format!("{}…", cut.trim_end())
}

/// The attributes of one tag, by name in lower case. The first of a repeated name wins.
fn parse_attributes(tag: &str) -> HashMap<String, String> {
    let mut attributes = HashMap::new();
    let body = tag.trim_start_matches('<').trim_start().trim_start_matches(|letter: char| letter.is_ascii_alphabetic());
    let body = body.strip_suffix('>').unwrap_or(body);
    let letters: Vec<char> = body.chars().collect();
    let mut at = 0;
    while at < letters.len() {
        while at < letters.len() && (letters[at].is_whitespace() || letters[at] == '/') {
            at += 1;
        }
        let start = at;
        while at < letters.len() && !letters[at].is_whitespace() && !"=/>\"'".contains(letters[at]) {
            at += 1;
        }
        if at == start {
            at += 1;
            continue;
        }
        let name: String = letters[start..at].iter().collect::<String>().to_lowercase();
        let mut value = String::new();
        let mut look = at;
        while look < letters.len() && letters[look].is_whitespace() {
            look += 1;
        }
        if look < letters.len() && letters[look] == '=' {
            look += 1;
            while look < letters.len() && letters[look].is_whitespace() {
                look += 1;
            }
            if look < letters.len() && (letters[look] == '"' || letters[look] == '\'') {
                let quote = letters[look];
                let from = look + 1;
                let to = letters[from..].iter().position(|letter| *letter == quote).map_or(letters.len(), |found| from + found);
                value = letters[from..to].iter().collect();
                look = (to + 1).min(letters.len());
            } else {
                let from = look;
                while look < letters.len() && !letters[look].is_whitespace() && !"\"'=<>`".contains(letters[look]) {
                    look += 1;
                }
                value = letters[from..look].iter().collect();
            }
            at = look;
        }
        attributes.entry(name).or_insert(value);
    }
    attributes
}

/// What the head of a page declares: its `<meta>` values by name, and its title.
pub fn extract_meta(html: &str) -> (HashMap<String, String>, String) {
    // Lower case in ASCII keeps every position where it was, so the two texts can be cut alike.
    let lower = html.to_ascii_lowercase();
    let end = lower.find("</head").unwrap_or(html.len());
    let (head, lower) = (&html[..end], &lower[..end]);
    let mut meta = HashMap::new();
    let mut from = 0;
    while let Some(found) = lower[from..].find("<meta") {
        let start = from + found;
        let Some(close) = lower[start..].find('>') else { break };
        let finish = start + close + 1;
        from = finish;
        let boundary = lower[start + 5..].chars().next().is_some_and(|letter| letter.is_whitespace() || letter == '/' || letter == '>');
        if !boundary {
            continue;
        }
        let attributes = parse_attributes(&head[start..finish]);
        let key = ["property", "name", "itemprop"].iter().find_map(|name| attributes.get(*name).filter(|value| !value.is_empty())).map(|key| key.to_lowercase());
        if let (Some(key), Some(content)) = (key, attributes.get("content").filter(|content| !content.is_empty())) {
            meta.entry(key).or_insert_with(|| content.clone());
        }
    }
    let title = lower.find("<title").and_then(|start| {
        let open = start + lower[start..].find('>')? + 1;
        let close = open + lower[open..].find("</title")?;
        Some(head[open..close].to_owned())
    });
    (meta, title.unwrap_or_default())
}

fn decode_html(data: &[u8], content_type: &str) -> String {
    let charset_in = |text: &str| {
        let lower = text.to_ascii_lowercase();
        let start = lower.find("charset=")? + 8;
        let name: String = lower[start..].trim_start_matches(['"', '\'']).chars().take_while(|letter| letter.is_ascii_alphanumeric() || *letter == '-' || *letter == '_').collect();
        (!name.is_empty()).then_some(name)
    };
    let start: String = data.iter().take(4096).map(|byte| char::from(*byte)).collect();
    let charset = charset_in(content_type).or_else(|| charset_in(&start)).unwrap_or_default();
    if matches!(charset.as_str(), "iso-8859-1" | "latin1" | "windows-1252" | "iso8859-1") {
        // Every byte of Latin-1 is the letter with the same number.
        return data.iter().map(|byte| char::from(*byte)).collect();
    }
    String::from_utf8_lossy(data).into_owned()
}

/// True for an address inside a home or office network, or of the computer itself.
pub fn is_private_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            let [a, b, ..] = address.octets();
            a == 0 || a == 10 || a == 127 || a >= 224 || (a == 169 && b == 254) || (a == 172 && (16..=31).contains(&b)) || (a == 192 && b == 168) || (a == 100 && (64..=127).contains(&b))
        }
        IpAddr::V6(address) => {
            if let Some(mapped) = address.to_ipv4_mapped() {
                return is_private_address(IpAddr::V4(mapped));
            }
            let first = address.segments()[0];
            address.is_unspecified() || address.is_loopback() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80 || (first & 0xff00) == 0xff00
        }
    }
}

/// A preview never fetches an address of the local network: a message cannot make anyone's
/// computer send requests to the router or to other machines of the house.
async fn assert_public(url: &Url) -> Result<(), String> {
    let local = || "endereço local".to_owned();
    match url.host() {
        Some(url::Host::Ipv4(address)) => (!is_private_address(IpAddr::V4(address))).then_some(()).ok_or_else(local),
        Some(url::Host::Ipv6(address)) => (!is_private_address(IpAddr::V6(address))).then_some(()).ok_or_else(local),
        Some(url::Host::Domain(host)) => {
            let host = host.to_lowercase();
            if !host.contains('.') || host.ends_with(".localhost") || host.ends_with(".local") || host.ends_with(".lan") {
                return Err(local());
            }
            let addresses: Vec<_> = tokio::net::lookup_host((host.as_str(), 80)).await.map_err(|error| error.to_string())?.collect();
            if addresses.is_empty() || addresses.iter().any(|address| is_private_address(address.ip())) {
                return Err(local());
            }
            Ok(())
        }
        None => Err(local()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewImage {
    pub data: Vec<u8>,
    pub mime: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub url: String,
    pub site_name: String,
    pub title: String,
    pub description: String,
    pub image: Option<PreviewImage>,
}

struct Fetcher {
    client: reqwest::Client,
    /// Only the tests fetch from this computer.
    allow_local: bool,
}

impl Fetcher {
    /// Follows redirects by hand, so every address on the way is checked before it is asked.
    async fn request(&self, url: Url, accept: &str) -> Result<(reqwest::Response, Url), String> {
        let mut current = url;
        for _ in 0..=MAX_REDIRECTS {
            if current.scheme() != "http" && current.scheme() != "https" {
                return Err("protocolo não suportado".into());
            }
            if !self.allow_local {
                assert_public(&current).await?;
            }
            let response = self
                .client
                .get(current.clone())
                .header(reqwest::header::ACCEPT, accept)
                .header(reqwest::header::ACCEPT_LANGUAGE, "pt-BR,pt;q=0.9,en;q=0.7")
                .send()
                .await
                .map_err(|error| error.to_string())?;
            let location = response.headers().get(reqwest::header::LOCATION).and_then(|value| value.to_str().ok()).map(str::to_owned);
            match location.filter(|_| response.status().is_redirection()) {
                Some(location) => current = current.join(&location).map_err(|error| error.to_string())?,
                None => return Ok((response, current)),
            }
        }
        Err("redirecionamentos demais".into())
    }

    async fn fetch_image(&self, url: Url) -> Option<PreviewImage> {
        let (response, _) = self.request(url, "image/webp,image/png,image/jpeg,image/gif;q=0.9").await.ok()?;
        read_image(response).await
    }

    async fn fetch(&self, address: &str) -> Option<Preview> {
        let url = Url::parse(address).ok()?;
        let (response, landed) = self.request(url, "text/html,application/xhtml+xml;q=0.9,image/*;q=0.8,*/*;q=0.5").await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        let content_type = response.headers().get(reqwest::header::CONTENT_TYPE).and_then(|value| value.to_str().ok()).unwrap_or("").to_owned();
        let site = landed.host_str().map(|host| host.strip_prefix("www.").unwrap_or(host).to_owned()).unwrap_or_default();
        if image_type(&content_type).is_some() {
            let image = read_image(response).await?;
            let name = landed.path_segments().and_then(|mut parts| parts.rfind(|part| !part.is_empty())).unwrap_or("");
            let name = percent_decode(name);
            return Some(Preview { url: address.to_owned(), site_name: site, title: clean_text(&name, MAX_TITLE), description: String::new(), image: Some(image) });
        }
        if !content_type.to_ascii_lowercase().contains("html") {
            return None;
        }
        let (data, _) = read_body(response, MAX_HTML_BYTES, true).await;
        let (meta, title) = extract_meta(&decode_html(&data, &content_type));
        let pick = |keys: &[&str]| keys.iter().find_map(|key| meta.get(*key).filter(|value| !value.is_empty())).cloned().unwrap_or_default();
        let or = |first: String, second: &str| if first.is_empty() { second.to_owned() } else { first };
        let mut preview = Preview {
            url: address.to_owned(),
            site_name: clean_text(&or(pick(&["og:site_name", "application-name"]), &site), MAX_SITE),
            title: clean_text(&or(pick(&["og:title", "twitter:title"]), &title), MAX_TITLE),
            description: clean_text(&pick(&["og:description", "twitter:description", "description"]), MAX_DESCRIPTION),
            image: None,
        };
        let image_address = decode_entities(pick(&["og:image:secure_url", "og:image:url", "og:image", "twitter:image", "twitter:image:src"]).trim());
        if !image_address.is_empty() {
            if let Ok(image_url) = landed.join(&image_address) {
                preview.image = self.fetch_image(image_url).await;
            }
        }
        (!preview.title.is_empty() || !preview.description.is_empty() || preview.image.is_some()).then_some(preview)
    }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let hex = (bytes[at] == b'%').then(||std::str::from_utf8(bytes.get(at + 1..at + 3)?).ok().and_then(|pair| u8::from_str_radix(pair, 16).ok())).flatten();
        match hex {
            Some(byte) => {
                out.push(byte);
                at += 3;
            }
            None => {
                out.push(bytes[at]);
                at += 1;
            }
        }
    }
    // An escape that does not make text leaves the name as it came in the address.
    String::from_utf8(out).unwrap_or_else(|_| text.to_owned())
}

fn image_type(content_type: &str) -> Option<&'static str> {
    let kind = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    IMAGE_TYPES.into_iter().find(|known| *known == kind)
}

/// Reads at most `limit` bytes. The flag tells whether the body ended before the limit.
async fn read_body(mut response: reqwest::Response, limit: usize, until_head_ends: bool) -> (Vec<u8>, bool) {
    let mut data = Vec::new();
    while let Ok(Some(chunk)) = response.chunk().await {
        data.extend_from_slice(&chunk);
        if data.len() >= limit {
            data.truncate(limit);
            return (data, false);
        }
        // The rest of a page is of no use once its head is whole.
        if until_head_ends && chunk.to_ascii_lowercase().windows(6).any(|window| window == b"</head") {
            break;
        }
    }
    (data, true)
}

async fn read_image(response: reqwest::Response) -> Option<PreviewImage> {
    let content_type = response.headers().get(reqwest::header::CONTENT_TYPE).and_then(|value| value.to_str().ok()).unwrap_or("").to_owned();
    let mime = image_type(&content_type)?;
    if !response.status().is_success() {
        return None;
    }
    let (data, complete) = read_body(response, MAX_IMAGE_BYTES, false).await;
    (complete && !data.is_empty()).then(|| PreviewImage { data, mime: mime.to_owned() })
}

/// Fetches previews and remembers the latest ones for a while, so typing around a link does not
/// ask its page again and again.
pub struct LinkPreviews {
    fetcher: Fetcher,
    cache: Mutex<HashMap<String, (Instant, Option<Preview>)>>,
}

impl Default for LinkPreviews {
    fn default() -> Self {
        Self::new(false)
    }
}

impl LinkPreviews {
    fn new(allow_local: bool) -> Self {
        let client = reqwest::Client::builder().user_agent(USER_AGENT).redirect(reqwest::redirect::Policy::none()).build().unwrap_or_default();
        Self { fetcher: Fetcher { client, allow_local }, cache: Mutex::new(HashMap::new()) }
    }

    /// The preview of a link, or nothing when the page has none or cannot be reached.
    pub async fn get(&self, address: &str) -> Option<Preview> {
        let url = Url::parse(address).ok()?;
        if (url.scheme() != "http" && url.scheme() != "https") || url.as_str().len() > MAX_URL_LENGTH {
            return None;
        }
        let cached = {
            let cache = self.cache.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            cache.get(address).filter(|(at, _)| at.elapsed() < CACHE_TTL).map(|(_, preview)| preview.clone())
        };
        if let Some(preview) = cached {
            return preview;
        }
        let preview = tokio::time::timeout(TIMEOUT, self.fetcher.fetch(address)).await.ok().flatten();
        let mut cache = self.cache.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if cache.len() >= CACHE_LIMIT {
            let oldest = cache.iter().min_by_key(|(_, (at, _))| *at).map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                cache.remove(&oldest);
            }
        }
        cache.insert(address.to_owned(), (Instant::now(), preview.clone()));
        preview
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    #[test]
    fn entities_become_letters_and_unknown_ones_stay() {
        assert_eq!(decode_entities("P&atilde;o &amp; caf&#233; &#x2014; &Eacute; &quot;bom&quot;"), "Pão & café — É \"bom\"");
        assert_eq!(decode_entities("a &desconhecida; b & c &#0; &;"), "a &desconhecida; b & c &#0; &;");
        assert_eq!(clean_text("  muito \n\t espaço  ", 50), "muito espaço");
        assert_eq!(clean_text("abcdefghij", 6), "abcde…");
    }

    #[test]
    fn the_head_of_a_page_gives_its_meta_values_and_title() {
        let html = r#"<!doctype html><HTML><head>
            <meta charset="utf-8">
            <META Property="og:title" Content="Bolo de &quot;cenoura&quot;">
            <meta content='Fofinho e rápido' name=description />
            <meta property="og:title" content="o segundo não vale">
            <meta name="vazio" content="">
            <metadata name="og:image" content="nao e uma meta">
            <title lang="pt">  Receita
              de bolo </title>
            </head><body><meta property="og:image" content="fora do head"></body></html>"#;
        let (meta, title) = extract_meta(html);
        assert_eq!(meta.get("og:title").map(String::as_str), Some("Bolo de &quot;cenoura&quot;"));
        assert_eq!(meta.get("description").map(String::as_str), Some("Fofinho e rápido"));
        assert!(!meta.contains_key("vazio") && !meta.contains_key("og:image"));
        assert_eq!(clean_text(&title, 200), "Receita de bolo");
    }

    #[test]
    fn addresses_of_the_house_are_told_apart_from_the_internet() {
        for local in ["127.0.0.1", "10.1.2.3", "192.168.0.1", "172.20.0.1", "169.254.1.1", "100.64.0.1", "0.0.0.0", "224.0.0.1", "::1", "::", "fc00::1", "fe80::1", "ff02::1", "::ffff:192.168.1.1"] {
            assert!(is_private_address(local.parse().unwrap()), "{local}");
        }
        for public in ["8.8.8.8", "172.32.0.1", "100.128.0.1", "2606:4700::1111", "::ffff:8.8.8.8"] {
            assert!(!is_private_address(public.parse().unwrap()), "{public}");
        }
    }

    #[tokio::test]
    async fn a_local_address_is_never_fetched() {
        let previews = LinkPreviews::default();
        for address in ["http://127.0.0.1:9/", "http://192.168.0.1/", "http://roteador/", "http://impressora.local/", "ftp://example.com/x", "not a link"] {
            assert_eq!(previews.get(address).await, None, "{address}");
        }
    }

    /// Run by hand: `cargo test -p telinha-core --lib linkpreview -- --ignored --nocapture`.
    /// Asks two real pages, over HTTPS and through the check of public addresses.
    #[tokio::test]
    #[ignore]
    async fn real_pages_give_previews() {
        let previews = LinkPreviews::default();
        let plain = previews.get("https://example.com/").await.expect("example.com answers");
        println!("{} | {} | {}", plain.site_name, plain.title, plain.description);
        assert_eq!(plain.title, "Example Domain");
        let rich = previews.get("https://github.com/rust-lang/rust").await.expect("github answers");
        println!("{} | {} | picture: {:?}", rich.site_name, rich.title, rich.image.as_ref().map(|image| (image.mime.clone(), image.data.len())));
        assert!(rich.title.contains("rust") && rich.image.is_some());
    }

    /// A page on this machine that redirects once, declares a preview in Latin-1 and has a
    /// picture next to it.
    async fn serve() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { return };
                tokio::spawn(async move {
                    let mut request = vec![0u8; 4096];
                    let read = socket.read(&mut request).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&request[..read]).into_owned();
                    let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
                    let reply = |status: &str, kind: &str, extra: &str, body: &[u8]| {
                        let mut reply = format!("HTTP/1.1 {status}\r\nContent-Type: {kind}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
                        reply.extend_from_slice(body);
                        reply
                    };
                    let page: Vec<u8> = "<html><head><meta property='og:title' content='Caf\u{e9} &amp; p\u{e3}o'><meta name=description content='Uma  receita'>\
                        <meta property='og:image' content='/img/foto.png'><title>outro</title></head><body>corpo</body></html>"
                        .chars()
                        .map(|letter| letter as u8)
                        .collect();
                    let reply = match path.as_str() {
                        "/vai" => reply("302 Found", "text/plain", "Location: /receita\r\n", b""),
                        "/receita" => reply("200 OK", "text/html; charset=ISO-8859-1", "", &page),
                        "/img/foto.png" | "/solta%20foto.png" => reply("200 OK", "image/png", "", b"\x89PNG fake"),
                        "/texto" => reply("200 OK", "text/plain", "", b"nada para mostrar"),
                        _ => reply("404 Not Found", "text/plain", "", b""),
                    };
                    let _ = socket.write_all(&reply).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        base
    }

    #[tokio::test]
    async fn a_page_gives_its_title_description_and_picture() {
        let base = serve().await;
        let previews = LinkPreviews::new(true);
        let address = format!("{base}/vai");
        let preview = previews.get(&address).await.unwrap();
        assert_eq!(preview.url, address);
        assert_eq!((preview.title.as_str(), preview.description.as_str(), preview.site_name.as_str()), ("Café & pão", "Uma receita", "127.0.0.1"));
        assert_eq!(preview.image, Some(PreviewImage { data: b"\x89PNG fake".to_vec(), mime: "image/png".into() }));

        let picture = previews.get(&format!("{base}/solta%20foto.png")).await.unwrap();
        assert_eq!((picture.title.as_str(), picture.image.is_some()), ("solta foto.png", true));
        assert_eq!(previews.get(&format!("{base}/texto")).await, None);
        assert_eq!(previews.get(&format!("{base}/sumiu")).await, None);
        // The second ask comes from memory.
        assert_eq!(previews.get(&address).await.unwrap().title, "Café & pão");
    }
}
