//! Finding an event's video call: Google Meet's own fields first, then a
//! Zoom, Teams, Meet or Webex address written into the location or the
//! description (often HTML, often with the link inside `<…>`).

/// A link that joins a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meeting {
    pub url: String,
    /// "Google Meet", "Zoom", ...
    pub service: &'static str,
}

/// The event's call: a video entry point, else `hangoutLink`, else the
/// first known call address in the location, then the description.
pub fn find(
    entry_points: &[(String, String)],
    hangout_link: Option<&str>,
    location: Option<&str>,
    description: Option<&str>,
) -> Option<Meeting> {
    let video = entry_points
        .iter()
        .find(|(kind, uri)| kind == "video" && is_web(uri))
        .map(|(_, uri)| uri.as_str());
    if let Some(url) = video.or(hangout_link.filter(|u| is_web(u))) {
        return Some(Meeting {
            url: url.to_owned(),
            service: service_of(url).unwrap_or("Video call"),
        });
    }
    [location, description]
        .into_iter()
        .flatten()
        .find_map(find_in_text)
}

/// The first call address in free text.
pub fn find_in_text(text: &str) -> Option<Meeting> {
    urls(text).find_map(|url| {
        service_of(&url).map(|service| Meeting {
            url: url.clone(),
            service,
        })
    })
}

fn is_web(url: &str) -> bool {
    url.starts_with("https://")
}

/// Which call service an address belongs to, if any. Only addresses that
/// join a meeting count, not a vendor's home page.
pub fn service_of(url: &str) -> Option<&'static str> {
    let rest = url.strip_prefix("https://")?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.to_ascii_lowercase();
    let on = |domain: &str| host == domain || host.ends_with(&format!(".{domain}"));
    if host == "meet.google.com" && path.len() >= 3 {
        Some("Google Meet")
    } else if (on("zoom.us") || on("zoomgov.com"))
        && ["j/", "my/", "w/", "s/", "wc/join/"]
            .iter()
            .any(|p| path.starts_with(p))
    {
        Some("Zoom")
    } else if (host == "teams.microsoft.com" && path.starts_with("l/meetup-join/"))
        || (host == "teams.live.com" && path.starts_with("meet/"))
    {
        Some("Teams")
    } else if on("webex.com") && (path.starts_with("meet/") || path.contains("j.php")) {
        Some("Webex")
    } else {
        None
    }
}

/// Every `https://` address in `text`, cut at whitespace, quotes and
/// angle brackets, without trailing punctuation, `&amp;` decoded.
fn urls(text: &str) -> impl Iterator<Item = String> + '_ {
    text.match_indices("https://").map(move |(start, _)| {
        let rest = &text[start..];
        let end = rest
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '[' | ']'))
            .unwrap_or(rest.len());
        let url = rest[..end].trim_end_matches(['.', ',', ';', ':', ')', '!', '?']);
        url.replace("&amp;", "&")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn points(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(k, u)| (k.to_string(), u.to_string()))
            .collect()
    }

    #[test]
    fn google_meet_fields_come_first() {
        let found = find(
            &points(&[
                ("phone", "tel:+1-555-0100"),
                ("video", "https://meet.google.com/abc-defg-hij"),
            ]),
            Some("https://meet.google.com/zzz-zzzz-zzz"),
            Some("https://zoom.us/j/123"),
            None,
        )
        .unwrap();
        assert_eq!(found.url, "https://meet.google.com/abc-defg-hij");
        assert_eq!(found.service, "Google Meet");
        let hangout = find(&[], Some("https://meet.google.com/abc-defg-hij"), None, None).unwrap();
        assert_eq!(hangout.url, "https://meet.google.com/abc-defg-hij");
        // Another service's entry point still counts.
        let other = find(&points(&[("video", "https://example.com/call/1")]), None, None, None).unwrap();
        assert_eq!(other.service, "Video call");
    }

    #[test]
    fn finds_links_in_the_location_then_the_description() {
        let zoom = find(
            &[],
            None,
            Some("https://us02web.zoom.us/j/81234567890?pwd=abc.1"),
            Some("https://teams.microsoft.com/l/meetup-join/19%3ameeting"),
        )
        .unwrap();
        assert_eq!(zoom.url, "https://us02web.zoom.us/j/81234567890?pwd=abc.1");
        assert_eq!(zoom.service, "Zoom");
        let teams = find(
            &[],
            None,
            Some("Room 4"),
            Some("Join: <https://teams.microsoft.com/l/meetup-join/19%3ameeting_x%40thread.v2/0?context=%7b%22Tid%22%7d>\nMore info"),
        )
        .unwrap();
        assert_eq!(teams.service, "Teams");
        assert_eq!(
            teams.url,
            "https://teams.microsoft.com/l/meetup-join/19%3ameeting_x%40thread.v2/0?context=%7b%22Tid%22%7d"
        );
        assert_eq!(find(&[], None, Some("Room 4"), Some("Bring snacks.")), None);
    }

    #[test]
    fn reads_html_descriptions() {
        let html = r#"Agenda<br><a href="https://zoom.us/j/999?pwd=x&amp;from=addon">https://zoom.us/j/999?pwd=x&amp;from=addon</a>."#;
        let found = find_in_text(html).unwrap();
        assert_eq!(found.url, "https://zoom.us/j/999?pwd=x&from=addon");
        let trailing = find_in_text("Call: https://meet.google.com/abc-defg-hij.").unwrap();
        assert_eq!(trailing.url, "https://meet.google.com/abc-defg-hij");
        // A vendor's other pages, or a plain link first, are skipped.
        let skips = find_in_text("See https://zoom.us/pricing and https://example.com, then https://teams.live.com/meet/9312").unwrap();
        assert_eq!(skips.url, "https://teams.live.com/meet/9312");
    }

    #[test]
    fn knows_the_services() {
        assert_eq!(service_of("https://meet.google.com/abc-defg-hij"), Some("Google Meet"));
        assert_eq!(service_of("https://meet.google.com/"), None);
        assert_eq!(service_of("https://zoom.us/my/ana"), Some("Zoom"));
        assert_eq!(service_of("https://company.zoomgov.com/j/1"), Some("Zoom"));
        assert_eq!(service_of("https://notzoom.us/j/1"), None);
        assert_eq!(service_of("https://acme.webex.com/meet/ana"), Some("Webex"));
        assert_eq!(service_of("https://acme.webex.com/acme/j.php?MTID=m1"), Some("Webex"));
        assert_eq!(service_of("http://zoom.us/j/1"), None, "web links only");
        assert_eq!(service_of("https://teams.microsoft.com/"), None);
    }
}
