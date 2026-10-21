//! Final ordering of results: match quality, name-prefix bonuses, and how
//! often and recently each item was opened (frecency).
//!
//! Providers return raw fuzzy scores; all tuning lives here.

use super::{ItemKind, ResultItem};

/// Typing a result's whole name always puts it first...
const EXACT_TIER: u8 = 1;
/// ...after a calculation's answer, which is what you typed it for.
const ANSWER_TIER: u8 = 2;
/// Bonus when the name starts with the query ("cal" → Calculator).
const PREFIX_BONUS: f32 = 0.3;
/// Smaller bonus when any word starts with the query ("code" → VS Code).
const WORD_PREFIX_BONUS: f32 = 0.15;
/// Frecency at which the boost maxes out (about 20 recent opens).
const FRECENCY_CAP: f64 = 20.0;

/// Sorts best first. `frecency` returns the decayed open count for an id.
pub fn rank(items: &mut [ResultItem], query: &str, frecency: &dyn Fn(&str) -> f64) {
    let query = query.trim().to_lowercase();
    let mut keyed: Vec<(u8, f32, usize)> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            // Clipboard history keeps its own order (pinned, then newest):
            // when you copied something matters more than how it matches.
            if item.kind == ItemKind::Clip {
                return (0, item.score, i);
            }
            let title = item.title.to_lowercase();
            let tier = if item.kind == ItemKind::Answer {
                ANSWER_TIER
            } else {
                u8::from(title == query) * EXACT_TIER
            };
            let blended =
                (item.score + name_bonus(&title, &query)) * (1.0 + boost(frecency(&item.id)));
            (tier, blended, i)
        })
        .collect();
    keyed.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.total_cmp(&a.1)).then(a.2.cmp(&b.2)));

    let order: Vec<usize> = keyed.into_iter().map(|(_, _, i)| i).collect();
    let sorted: Vec<ResultItem> = order.iter().map(|&i| items[i].clone()).collect();
    items.clone_from_slice(&sorted);
}

fn name_bonus(title: &str, query: &str) -> f32 {
    if title.starts_with(query) {
        PREFIX_BONUS
    } else if title
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| !w.is_empty() && w.starts_with(query))
    {
        WORD_PREFIX_BONUS
    } else {
        0.0
    }
}

/// 0..=1 on a log scale: the first few opens matter most.
fn boost(frecency: f64) -> f32 {
    if frecency <= 0.0 {
        return 0.0;
    }
    ((1.0 + frecency).ln() / (1.0 + FRECENCY_CAP).ln()).min(1.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::Action;
    use std::collections::HashMap;

    fn item(id: &str, title: &str, score: f32) -> ResultItem {
        ResultItem {
            id: id.into(),
            kind: ItemKind::App,
            title: title.into(),
            subtitle: String::new(),
            icon: None,
            score,
            actions: vec![Action::Open],
            accessory: None,
            confirm: None,
        }
    }

    fn titles(items: &[ResultItem]) -> Vec<&str> {
        items.iter().map(|i| i.title.as_str()).collect()
    }

    fn none(_: &str) -> f64 {
        0.0
    }

    #[test]
    fn higher_score_first_without_history() {
        let mut items = vec![item("a", "Alpha", 0.2), item("b", "Beta", 0.8)];
        rank(&mut items, "zz", &none);
        assert_eq!(titles(&items), ["Beta", "Alpha"]);
    }

    #[test]
    fn exact_name_always_wins() {
        let mut items = vec![item("a", "Code Writer", 1.0), item("b", "Code", 0.5)];
        let heavy = |id: &str| if id == "a" { 100.0 } else { 0.0 };
        rank(&mut items, "code", &heavy);
        assert_eq!(titles(&items), ["Code", "Code Writer"]);
    }

    #[test]
    fn answers_come_before_even_an_exact_name() {
        let mut items = vec![
            item("a", "1+1", 1.0),
            ResultItem::new("calc:1+1", ItemKind::Answer, "2").with_score(0.1),
        ];
        rank(&mut items, "1+1", &none);
        assert_eq!(titles(&items), ["2", "1+1"]);
    }

    #[test]
    fn prefix_beats_equal_scores() {
        let mut items = vec![
            item("a", "Visual Studio Code", 0.8),
            item("b", "Calculator", 0.8),
        ];
        rank(&mut items, "c", &none);
        assert_eq!(titles(&items)[0], "Calculator");
    }

    #[test]
    fn frequent_use_can_beat_a_better_match() {
        let mut items = vec![
            item("calc", "Calculator", 0.9),
            item("vsc", "Visual Studio Code", 0.8),
        ];
        let history: HashMap<&str, f64> = [("vsc", 15.0)].into();
        let lookup = |id: &str| history.get(id).copied().unwrap_or(0.0);
        rank(&mut items, "c", &lookup);
        assert_eq!(titles(&items)[0], "Visual Studio Code");
    }

    #[test]
    fn ties_keep_provider_order() {
        let mut items = vec![item("a", "Same", 0.5), item("b", "Same", 0.5)];
        rank(&mut items, "zz", &none);
        assert_eq!(items[0].id, "a");
    }

    #[test]
    fn clips_keep_their_order() {
        let clip = |id: &str, title: &str, score: f32| {
            ResultItem::new(id, ItemKind::Clip, title).with_score(score)
        };
        // Neither a name match nor frequent pasting reorders the history.
        let mut items = vec![
            clip("clip:2", "newest", 1.0),
            clip("clip:1", "clip", 0.5),
        ];
        let heavy = |id: &str| if id == "clip:1" { 100.0 } else { 0.0 };
        rank(&mut items, "clip", &heavy);
        assert_eq!(titles(&items), ["newest", "clip"]);
    }

    #[test]
    fn boost_is_bounded() {
        assert_eq!(boost(0.0), 0.0);
        assert!(boost(1.0) > 0.0);
        assert_eq!(boost(1000.0), 1.0);
    }
}
