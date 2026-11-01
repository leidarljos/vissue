//! Repository tasks that belong to the release rather than to any crate.
//!
//! ```console
//! $ cargo run -q -p xtask -- publish-order [--json | --with-versions]
//! $ cargo run -q -p xtask -- bench-tracker [BINARY] [ISSUES]
//! ```

mod bench;

use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

use serde_json::Value;

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let task = args.next().unwrap_or_default();
    let flags: Vec<String> = args.collect();
    match task.as_str() {
        "publish-order" => publish_order(&flags),
        "bench-tracker" => bench::tracker(&flags),
        "-h" | "--help" | "help" | "" => {
            println!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown task: {other}\n\n{USAGE}")),
    }
}

const USAGE: &str = "xtask: repository tasks\n\
    \n\
    publish-order [--json|--with-versions]   publishable crates, dependencies first\n\
    bench-tracker [BINARY] [ISSUES]          what each verb costs at a given size";

/// The workspace's publishable crates, dependencies first.
///
/// `cargo publish` uploads one crate at a time and a crate cannot resolve
/// until everything it names is already on the registry, so the order is part
/// of the release. A hand-written list of it goes stale the first time a crate
/// is added, which is how a workspace that had grown from three members to
/// seven came to have a release workflow that named three.
fn publish_order(flags: &[String]) -> Result<(), String> {
    let (edges, versions) = workspace_members()?;
    let order = topological(&edges)?;
    if flags.iter().any(|f| f == "--json") {
        println!(
            "{}",
            serde_json::to_string(&order).map_err(|e| e.to_string())?
        );
    } else if flags.iter().any(|f| f == "--with-versions") {
        for name in &order {
            println!("{name} {}", versions[name]);
        }
    } else {
        for name in &order {
            println!("{name}");
        }
    }
    Ok(())
}

/// Which crates a crate names, keyed by crate name.
type Edges = BTreeMap<String, BTreeSet<String>>;

/// The version each crate would be uploaded at.
type Versions = BTreeMap<String, String>;

/// Internal dependency edges and versions, keyed by crate name.
fn workspace_members() -> Result<(Edges, Versions), String> {
    let out = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cargo metadata: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let meta: Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    let packages = meta["packages"]
        .as_array()
        .ok_or("cargo metadata: no packages")?;

    let names: BTreeSet<&str> = packages
        .iter()
        .filter_map(|pkg| pkg["name"].as_str())
        .collect();

    let mut edges = Edges::new();
    let mut versions = Versions::new();
    for pkg in packages {
        let Some(name) = pkg["name"].as_str() else {
            continue;
        };
        // `publish = false` opts a crate out and `publish = ["registry"]`
        // limits it. Either way an empty list means nowhere, which is the only
        // case that removes a crate from the release.
        if pkg["publish"].as_array().is_some_and(Vec::is_empty) {
            continue;
        }
        versions.insert(
            name.to_string(),
            pkg["version"].as_str().unwrap_or_default().to_string(),
        );
        let deps = pkg["dependencies"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter_map(|dep| {
                let dep_name = dep["name"].as_str()?;
                // A dev-dependency is not part of what a consumer resolves, so
                // it does not constrain the upload order.
                (names.contains(dep_name) && dep["kind"].as_str() != Some("dev"))
                    .then(|| dep_name.to_string())
            })
            .collect();
        edges.insert(name.to_string(), deps);
    }

    // Drop edges to crates that are not being published at all.
    let published: BTreeSet<String> = edges.keys().cloned().collect();
    for deps in edges.values_mut() {
        deps.retain(|dep| published.contains(dep));
    }
    Ok((edges, versions))
}

/// Dependencies first, ties broken by name so the order is reproducible.
fn topological(edges: &Edges) -> Result<Vec<String>, String> {
    let mut ordered: Vec<String> = Vec::with_capacity(edges.len());
    let mut placed: BTreeSet<String> = BTreeSet::new();
    let mut remaining = edges.clone();

    while !remaining.is_empty() {
        let ready: Vec<String> = remaining
            .iter()
            .filter(|(_, deps)| deps.is_subset(&placed))
            .map(|(name, _)| name.clone())
            .collect();
        if ready.is_empty() {
            let cycle: Vec<&str> = remaining.keys().map(String::as_str).collect();
            return Err(format!("dependency cycle among: {}", cycle.join(", ")));
        }
        for name in ready {
            remaining.remove(&name);
            placed.insert(name.clone());
            ordered.push(name);
        }
    }
    Ok(ordered)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edges(pairs: &[(&str, &[&str])]) -> Edges {
        pairs
            .iter()
            .map(|(name, deps)| {
                (
                    (*name).to_string(),
                    deps.iter().map(|d| (*d).to_string()).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn a_dependency_is_uploaded_before_what_names_it() {
        let order = topological(&edges(&[
            ("cli", &["core"]),
            ("core", &[]),
            ("serve", &["core"]),
        ]))
        .unwrap();
        let at = |name: &str| order.iter().position(|n| n == name).unwrap();
        assert!(at("core") < at("cli"));
        assert!(at("core") < at("serve"));
    }

    #[test]
    fn the_order_is_the_same_every_run() {
        let graph = edges(&[("b", &["a"]), ("a", &[]), ("c", &["a"]), ("d", &["b", "c"])]);
        assert_eq!(topological(&graph).unwrap(), topological(&graph).unwrap());
    }

    /// A cycle cannot be published in any order, so saying which crates are in
    /// it beats stopping halfway through an upload nobody can undo.
    #[test]
    fn a_cycle_is_named_rather_than_ordered() {
        let err = topological(&edges(&[("a", &["b"]), ("b", &["a"])])).unwrap_err();
        assert!(err.contains('a') && err.contains('b'), "{err}");
    }

    #[test]
    fn this_workspace_puts_core_first_and_lists_no_helper() {
        let (graph, versions) = workspace_members().unwrap();
        let order = topological(&graph).unwrap();
        assert_eq!(order.first().map(String::as_str), Some("vissue-core"));
        // `publish = false` is what keeps this crate out of the release.
        assert!(!order.iter().any(|name| name == "xtask"), "{order:?}");
        assert!(versions.values().all(|v| !v.is_empty()));
    }
}
