use anyhow::{anyhow, Context, Result};
use regex::Regex;
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

#[derive(Debug, PartialEq, Eq)]
struct SnapshotRefs {
    title: String,
    description: String,
    create_button: String,
}

struct TargetResult {
    target: String,
    result: Result<String, String>,
}

pub fn create(repo: &str, from: &str, to: &str, title: &str, body: &str) -> Result<()> {
    let targets = parse_targets(to)?;
    let repo = repo.trim_end_matches('/').to_string();
    let mut results = Vec::new();

    for target in targets {
        let result =
            create_for_target(&repo, from, &target, title, body).map_err(|e| format!("{e:#}"));
        results.push(TargetResult { target, result });
    }

    println!("gogs-pr summary:");
    for item in &results {
        match &item.result {
            Ok(url) => println!("{} -> {}", item.target, url),
            Err(err) => println!("{} -> ERROR: {}", item.target, err),
        }
    }

    if results.iter().any(|item| item.result.is_ok()) {
        Ok(())
    } else {
        Err(anyhow!("all Gogs pull request creations failed"))
    }
}

fn create_for_target(
    repo: &str,
    from: &str,
    target: &str,
    title: &str,
    body: &str,
) -> Result<String> {
    run_chrome_use(&["open", &format!("{repo}/compare/{target}...{from}")])
        .with_context(|| format!("opening compare page for target {target}"))?;
    sleep(Duration::from_secs(2));

    let snapshot = capture_chrome_use(&["snapshot", "-i"])
        .with_context(|| format!("capturing compare page snapshot for target {target}"))?;
    let refs = parse_snapshot_refs(&snapshot)
        .with_context(|| format!("finding form refs for target {target}"))?;

    run_chrome_use(&["fill", &format!("@{}", refs.title), title])
        .with_context(|| format!("filling Title for target {target}"))?;
    if !body.is_empty() {
        run_chrome_use(&["fill", &format!("@{}", refs.description), body])
            .with_context(|| format!("filling description for target {target}"))?;
    }

    let snapshot = capture_chrome_use(&["snapshot", "-i"])
        .with_context(|| format!("re-capturing compare page snapshot for target {target}"))?;
    let create_ref = find_create_button_ref(&snapshot)
        .with_context(|| format!("finding Create Pull Request button for target {target}"))?;

    run_chrome_use(&["click", &format!("@{create_ref}")])
        .with_context(|| format!("clicking Create Pull Request for target {target}"))?;
    sleep(Duration::from_secs(2));

    let href = capture_chrome_use(&["eval", "location.href"])
        .with_context(|| format!("reading resulting URL for target {target}"))?;
    parse_created_pr_url(&href).ok_or_else(|| {
        anyhow!(
            "target {target}: expected resulting URL to match */pulls/<number>, got {}",
            href.trim()
        )
    })
}

fn parse_targets(to: &str) -> Result<Vec<String>> {
    let targets: Vec<String> = to
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    if targets.is_empty() {
        return Err(anyhow!("--to must contain at least one target branch"));
    }
    Ok(targets)
}

fn run_chrome_use(args: &[&str]) -> Result<()> {
    capture_chrome_use(args).map(|_| ())
}

fn capture_chrome_use(args: &[&str]) -> Result<String> {
    let out = Command::new(crate::chrome_use::bin())
        .args(args)
        .output()
        .with_context(|| format!("running `chrome-use {}`", args.join(" ")))?;
    if !out.status.success() {
        return Err(anyhow!(
            "chrome-use {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn parse_snapshot_refs(snapshot: &str) -> Result<SnapshotRefs> {
    Ok(SnapshotRefs {
        title: find_title_ref(snapshot)?,
        description: find_description_ref(snapshot)?,
        create_button: find_create_button_ref(snapshot)?,
    })
}

fn find_title_ref(snapshot: &str) -> Result<String> {
    let ref_re = ref_regex();
    for line in snapshot.lines() {
        if line.contains("textbox \"Title\"") {
            return ref_re
                .captures(line)
                .and_then(|caps| caps.get(1))
                .map(|m| m.as_str().to_string())
                .ok_or_else(|| anyhow!("Title textbox is missing a ref"));
        }
    }
    Err(anyhow!("missing Title textbox ref"))
}

fn find_description_ref(snapshot: &str) -> Result<String> {
    let ref_re = ref_regex();
    let mut after_title = false;
    for line in snapshot.lines() {
        if line.contains("textbox \"Title\"") {
            after_title = true;
            continue;
        }
        if after_title && line.contains("textbox [ref=") {
            return ref_re
                .captures(line)
                .and_then(|caps| caps.get(1))
                .map(|m| m.as_str().to_string())
                .ok_or_else(|| anyhow!("description textbox is missing a ref"));
        }
    }
    Err(anyhow!(
        "missing unlabeled description textbox ref after Title"
    ))
}

fn find_create_button_ref(snapshot: &str) -> Result<String> {
    let ref_re = ref_regex();
    for line in snapshot.lines() {
        if line.contains("Create Pull Request") {
            return ref_re
                .captures(line)
                .and_then(|caps| caps.get(1))
                .map(|m| m.as_str().to_string())
                .ok_or_else(|| anyhow!("Create Pull Request button is missing a ref"));
        }
    }
    Err(anyhow!("missing Create Pull Request button ref"))
}

fn ref_regex() -> Regex {
    Regex::new(r"ref=(e\d+)").expect("valid ref regex")
}

fn parse_created_pr_url(stdout: &str) -> Option<String> {
    let re = Regex::new(r#"https?://[^\s\"]+/pulls/\d+"#).expect("valid pull request URL regex");
    re.find(stdout).map(|m| m.as_str().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SNAPSHOT: &str = r#"
- textbox "Title" [required, ref=e270]
- textbox [ref=e271]
- button "Create Pull Request" [ref=e272]
"#;

    #[test]
    fn finds_required_refs_in_snapshot() {
        let refs = parse_snapshot_refs(SNAPSHOT).unwrap();

        assert_eq!(refs.title, "e270");
        assert_eq!(refs.description, "e271");
        assert_eq!(refs.create_button, "e272");
    }

    #[test]
    fn reports_missing_title_ref() {
        let err = parse_snapshot_refs(
            r#"
- textbox [ref=e271]
- button "Create Pull Request" [ref=e272]
"#,
        )
        .unwrap_err()
        .to_string();

        assert!(err.contains("Title"));
    }

    #[test]
    fn extracts_created_pull_request_url_from_eval_output() {
        let url = parse_created_pr_url("\"https://sg-git.pwtk.cc/ka-cn/super-admin/pulls/123\"\n")
            .unwrap();

        assert_eq!(url, "https://sg-git.pwtk.cc/ka-cn/super-admin/pulls/123");
    }

    #[test]
    fn rejects_non_pull_request_eval_output() {
        assert!(parse_created_pr_url(
            "\"https://sg-git.pwtk.cc/ka-cn/super-admin/compare/dev...feat\"\n"
        )
        .is_none());
    }
}
