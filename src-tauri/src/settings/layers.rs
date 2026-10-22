//! The settings layers: defaults, then `settings.toml` (which can live in a
//! synced folder and be shared by several PCs), then `settings.local.toml`
//! (this PC only). Later layers win, key by key.
//!
//! Pure table work, so the split and the merge are easy to test.

use toml::{Table, Value};

/// Settings that belong to one PC. They're saved in `settings.local.toml`,
/// so a synced `settings.toml` never carries them to another PC:
/// - index folders and exclusions shape this PC's file index, and only
///   apply after a restart, which a sync shouldn't spring on you;
/// - the Claude Code and Codex locations and the scripts folder are paths
///   on this PC's disk;
/// - starting at login, the Windows key override and closing Start are
///   about how this PC is used and how its Windows behaves.
pub const MACHINE_KEYS: &[&str] = &[
    "general.launch_at_login",
    "hotkeys.win_key_override",
    "hotkeys.close_start",
    "search.index_folders",
    "search.exclusions",
    "ai.claude.path",
    "ai.chatgpt.path",
    "scripts.folder",
];

const VERSION: &str = "version";

/// The settings in effect: `synced` over `defaults`, then `local` over both.
pub fn layer(defaults: Table, synced: Table, local: Table) -> Table {
    let mut table = defaults;
    merge(&mut table, synced);
    merge(&mut table, local);
    table
}

/// Splits the current settings (`full`) into what each file should hold,
/// given what the files hold now.
///
/// - Machine keys go to the local file, and are taken out of the synced one
///   (older files had everything in one place).
/// - Any other key already in the local file stays there: adding a key to
///   `settings.local.toml` by hand makes it per-PC, and saving keeps it so.
///   The synced file keeps its own value for it, for the other PCs.
/// - Keys a file has that this version doesn't know (written by a newer
///   version on another PC) are kept, so saving here never drops them.
pub fn split_for_save(full: Table, on_disk_synced: Table, on_disk_local: Table) -> (Table, Table) {
    let mut synced = full;
    let mut local = Table::new();
    if let Some(version) = synced.get(VERSION) {
        local.insert(VERSION.into(), version.clone());
    }
    let own_keys = leaf_paths(&on_disk_local);
    let local_keys = MACHINE_KEYS
        .iter()
        .map(|k| (*k).to_owned())
        .chain(own_keys.into_iter().filter(|k| k != VERSION));
    for key in local_keys {
        if let Some(value) = take(&mut synced, &key) {
            put(&mut local, &key, value);
        }
    }

    let mut synced_out = on_disk_synced;
    merge(&mut synced_out, synced);
    for key in MACHINE_KEYS {
        take(&mut synced_out, key);
    }
    let mut local_out = on_disk_local;
    merge(&mut local_out, local);
    (synced_out, local_out)
}

/// Recursively overlays `over` onto `base`. Tables merge; other values
/// (including arrays) replace.
pub fn merge(base: &mut Table, over: Table) {
    for (key, value) in over {
        match (base.get_mut(&key), value) {
            (Some(Value::Table(b)), Value::Table(o)) => merge(b, o),
            (_, v) => {
                base.insert(key, v);
            }
        }
    }
}

/// The dotted path of every value in `table` that isn't itself a table
/// ("search.index_folders"). Arrays count as one value.
pub fn leaf_paths(table: &Table) -> Vec<String> {
    let mut paths = Vec::new();
    collect_leaves(table, "", &mut paths);
    paths
}

fn collect_leaves(table: &Table, prefix: &str, out: &mut Vec<String>) {
    for (key, value) in table {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match value {
            Value::Table(inner) => collect_leaves(inner, &path, out),
            _ => out.push(path),
        }
    }
}

/// Removes the value at a dotted path, dropping tables it leaves empty.
fn take(table: &mut Table, path: &str) -> Option<Value> {
    match path.split_once('.') {
        None => table.remove(path),
        Some((head, rest)) => {
            let Some(Value::Table(inner)) = table.get_mut(head) else {
                return None;
            };
            let value = take(inner, rest);
            if inner.is_empty() {
                table.remove(head);
            }
            value
        }
    }
}

/// Sets the value at a dotted path, making tables on the way.
fn put(table: &mut Table, path: &str, value: Value) {
    match path.split_once('.') {
        None => {
            table.insert(path.to_owned(), value);
        }
        Some((head, rest)) => {
            let entry = table
                .entry(head.to_owned())
                .or_insert_with(|| Value::Table(Table::new()));
            if !entry.is_table() {
                *entry = Value::Table(Table::new());
            }
            if let Value::Table(inner) = entry {
                put(inner, rest, value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;

    fn table(text: &str) -> Table {
        toml::from_str(text).unwrap()
    }

    fn defaults() -> Table {
        Table::try_from(Settings::default()).unwrap()
    }

    fn custom() -> Settings {
        let mut s = Settings::default();
        s.hotkeys.toggle = "Alt+Space".into();
        s.hotkeys.win_key_override = false;
        s.search.index_folders = vec![r"D:\Projects".into()];
        s.search.max_results = 6;
        s.ai.claude.path = r"C:\tools\claude.exe".into();
        s.ai.claude.model = "haiku".into();
        s.general.launch_at_login = true;
        s.appearance.tint = "purple".into();
        s
    }

    fn get<'a>(table: &'a Table, path: &str) -> Option<&'a Value> {
        let mut value: Option<&Value> = None;
        let mut current = table;
        for part in path.split('.') {
            value = current.get(part);
            if let Some(Value::Table(inner)) = value {
                current = inner;
            }
        }
        value
    }

    #[test]
    fn machine_keys_exist_in_settings() {
        let all = leaf_paths(&defaults());
        for key in MACHINE_KEYS {
            assert!(all.iter().any(|k| k == key), "{key} is not a setting");
        }
    }

    #[test]
    fn machine_keys_go_to_the_local_file() {
        let full = Table::try_from(custom()).unwrap();
        let (synced, local) = split_for_save(full, Table::new(), Table::new());
        for key in MACHINE_KEYS {
            assert!(get(&synced, key).is_none(), "{key} in the synced file");
            assert!(get(&local, key).is_some(), "{key} not in the local file");
        }
        assert_eq!(
            get(&synced, "hotkeys.toggle").unwrap().as_str(),
            Some("Alt+Space")
        );
        assert!(get(&local, "hotkeys.toggle").is_none());
        assert_eq!(get(&synced, "version"), get(&local, "version"));
        // `general` only holds a machine key: no empty section left behind.
        assert!(!synced.contains_key("general"));
    }

    #[test]
    fn split_then_layer_gives_the_same_settings() {
        let s = custom();
        let (synced, local) =
            split_for_save(Table::try_from(&s).unwrap(), Table::new(), Table::new());
        let back: Settings = layer(defaults(), synced, local).try_into().unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn a_single_old_file_moves_its_machine_keys() {
        // Before the split, everything was in settings.toml. Loading it
        // with no local file keeps every value...
        let old = Table::try_from(custom()).unwrap();
        let loaded: Settings = layer(defaults(), old.clone(), Table::new())
            .try_into()
            .unwrap();
        assert_eq!(loaded, custom());
        // ...and the first save moves this PC's values to the local file.
        let (synced, local) = split_for_save(Table::try_from(&loaded).unwrap(), old, Table::new());
        assert!(get(&synced, "search.index_folders").is_none());
        assert_eq!(
            get(&local, "search.index_folders"),
            Some(&Value::Array(vec![r"D:\Projects".into()]))
        );
        assert_eq!(
            get(&synced, "search.max_results").unwrap().as_integer(),
            Some(6)
        );
    }

    #[test]
    fn local_values_win_over_synced_ones() {
        let synced = table("[search]\nindex_folders = ['C:\\Other']\nmax_results = 5\n");
        let local = table("[search]\nindex_folders = ['D:\\Mine']\n");
        let s: Settings = layer(defaults(), synced, local).try_into().unwrap();
        assert_eq!(s.search.index_folders, [r"D:\Mine"]);
        assert_eq!(s.search.max_results, 5);
    }

    #[test]
    fn keys_added_to_the_local_file_by_hand_stay_local() {
        // This PC wants its own tint; the synced file keeps the shared one.
        let on_disk_synced = table("[appearance]\ntint = 'green'\n");
        let on_disk_local = table("version = 1\n[appearance]\ntint = 'red'\n");
        let mut s: Settings = layer(defaults(), on_disk_synced.clone(), on_disk_local.clone())
            .try_into()
            .unwrap();
        assert_eq!(s.appearance.tint, "red");
        s.appearance.tint = "orange".into();
        let (synced, local) =
            split_for_save(Table::try_from(&s).unwrap(), on_disk_synced, on_disk_local);
        assert_eq!(
            get(&local, "appearance.tint").unwrap().as_str(),
            Some("orange")
        );
        assert_eq!(
            get(&synced, "appearance.tint").unwrap().as_str(),
            Some("green")
        );
    }

    #[test]
    fn unknown_keys_from_a_newer_version_survive_a_save() {
        let on_disk_synced =
            table("future = 1\n[clipboard]\nnew_option = true\n[brand_new]\nx = 'y'\n");
        let on_disk_local = table("[hotkeys]\nfuture_local = 2\n");
        let full = Table::try_from(custom()).unwrap();
        let (synced, local) = split_for_save(full, on_disk_synced, on_disk_local);
        assert_eq!(get(&synced, "future").unwrap().as_integer(), Some(1));
        assert_eq!(
            get(&synced, "clipboard.new_option").unwrap().as_bool(),
            Some(true)
        );
        assert_eq!(get(&synced, "brand_new.x").unwrap().as_str(), Some("y"));
        assert_eq!(
            get(&local, "hotkeys.future_local").unwrap().as_integer(),
            Some(2)
        );
        // ...while known values are this PC's current ones.
        assert_eq!(
            get(&synced, "clipboard.enabled").unwrap().as_bool(),
            Some(true)
        );
    }

    #[test]
    fn arrays_replace_rather_than_merge() {
        let mut base = table("a = [1, 2, 3]\n[t]\nb = 1\nc = 2\n");
        merge(&mut base, table("a = [4]\n[t]\nc = 3\n"));
        assert_eq!(base, table("a = [4]\n[t]\nb = 1\nc = 3\n"));
    }

    #[test]
    fn take_and_put_follow_dotted_paths() {
        let mut t = table("[a.b]\nc = 1\n[a]\nd = 2\n");
        assert_eq!(take(&mut t, "a.b.c").unwrap().as_integer(), Some(1));
        assert_eq!(t, table("[a]\nd = 2\n"));
        assert_eq!(take(&mut t, "a.d.x"), None);
        assert_eq!(take(&mut t, "missing.key"), None);
        put(&mut t, "x.y.z", Value::Boolean(true));
        put(&mut t, "a.d.e", Value::Integer(3));
        assert_eq!(t, table("[x.y]\nz = true\n[a.d]\ne = 3\n"));
    }

    #[test]
    fn leaf_paths_are_dotted() {
        let mut paths = leaf_paths(&table("v = 1\n[a]\nb = [1]\n[a.c]\nd = 'x'\n"));
        paths.sort();
        assert_eq!(paths, ["a.b", "a.c.d", "v"]);
    }
}
