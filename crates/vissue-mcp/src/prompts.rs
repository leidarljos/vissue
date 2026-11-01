//! Prompts: the sequences that start in the tracker.

use rmcp::{
    ErrorData as McpError, handler::server::wrapper::Parameters, model::*, prompt, prompt_router,
};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::server::VissueServer;

/// What a handover is a slice of.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SliceArgs {
    /// Projects to take whole, comma separated. Either this or `issues`.
    pub projects: Option<String>,
    /// Issue ids to take, comma separated. Their blockers and plans come too.
    pub issues: Option<String>,
    /// Directory to write the bag into. Defaults to `./satchel`.
    pub out: Option<String>,
}

/// Where to look for the next thing to do.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadyArgs {
    /// One project, or every project when absent.
    pub project: Option<String>,
}

/// One issue, by id.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct IssueArgs {
    /// The issue id, such as `vissue-ab12`.
    pub issue: String,
}

fn asked(text: String) -> Vec<PromptMessage> {
    vec![PromptMessage::new_text(Role::User, text)]
}

#[prompt_router(vis = "pub(crate)")]
impl VissueServer {
    /// Pack a slice of this seat so somebody else can open it: the issues, what
    /// they stand on, the deeds they cite and what the seat learned.
    #[prompt(name = "pack_a_slice")]
    pub async fn pack_a_slice_prompt(
        &self,
        Parameters(args): Parameters<SliceArgs>,
    ) -> Result<Vec<PromptMessage>, McpError> {
        let projects = args.projects.unwrap_or_default();
        let issues = args.issues.unwrap_or_default();
        if projects.trim().is_empty() && issues.trim().is_empty() {
            return Err(McpError::invalid_params(
                "a slice that names nothing is not a slice: give projects, issues, or both",
                None,
            ));
        }
        let out = args.out.unwrap_or_else(|| "./satchel".into());
        let named = match (projects.trim(), issues.trim()) {
            ("", one) => format!("issues {one}"),
            (one, "") => format!("projects {one}"),
            (p, i) => format!("projects {p} and issues {i}"),
        };
        Ok(asked(format!(
            "Pack a handover of {named} into {out}.\n\
             \n\
             Four stores hold four different things and the bag needs all of them, so\n\
             the order matters:\n\
             \n\
             1. `vissue_satchel` with what was named. It walks the closure first: an\n\
                issue whose blockers are absent is a task with no account of why it is\n\
                not done, and a child with no parent is a task with no reason. What\n\
                comes back names the deed accessions the issues cite, and that list is\n\
                the only thing the other two stores need from the tracker.\n\
             2. The pack's atoms and the deed store's bytes, both keyed by that list.\n\
                Deeds carry an inclusion proof; atoms do not, and that asymmetry is\n\
                real rather than an omission.\n\
             3. `vissue_satchel_seal`, after the other two have written, so the\n\
                manifest covers the whole payload rather than the half the tracker\n\
                wrote. Sealing first proves only what was enumerated first.\n\
             4. Sign the manifest. A receiver who recomputes digests from the bag they\n\
                were handed is checking the bag against itself; the signature is what\n\
                makes it a statement by somebody.\n\
             \n\
             Then `vissue_satchel_verify` and report what it says it did not check,\n\
             not only that it passed."
        )))
    }

    /// Take the next piece of work: what is actionable, what it stands on, and
    /// what the seat already knows about it.
    #[prompt(name = "pick_up_work")]
    pub async fn pick_up_work_prompt(
        &self,
        Parameters(args): Parameters<ReadyArgs>,
    ) -> Result<Vec<PromptMessage>, McpError> {
        let scope = args
            .project
            .filter(|p| !p.trim().is_empty())
            .map_or_else(|| "every project".to_string(), |p| format!("project {p}"));
        Ok(asked(format!(
            "Find the next thing worth doing in {scope} and pick it up.\n\
             \n\
             `vissue_ready` rather than `vissue_list`: ready is TODO or STARTED with no\n\
             open blocker, which is a different set from open, and working the second\n\
             is how a blocked task gets started twice.\n\
             \n\
             Before claiming, read the issue and `vissue_recall` it. Recall answers what\n\
             the work stands on and what its inputs produced, which is the half of the\n\
             context the issue body does not carry.\n\
             \n\
             Claim it, and say so in one line: a claim is a statement to whoever else is\n\
             holding a seat, not a private note. Then do the work, cite what it produced\n\
             with `vissue_deed`, and close it. An issue closed with no deed cited is a\n\
             claim the tracker cannot stand behind."
        )))
    }

    /// Check whether an issue's citations still hold: still evidenced, still
    /// the tip, and what stands on anything that moved.
    #[prompt(name = "check_citations")]
    pub async fn check_citations_prompt(
        &self,
        Parameters(args): Parameters<IssueArgs>,
    ) -> Result<Vec<PromptMessage>, McpError> {
        let issue = args.issue;
        Ok(asked(format!(
            "Check the deeds {issue} cites, and say what still holds.\n\
             \n\
             `vissue_recall {issue}` gives the accessions. Three questions follow and\n\
             they are not the same question:\n\
             \n\
             - does the deed store still answer for the bytes it published. A deed that\n\
               was deleted leaves every signature over it perfectly good; only the log\n\
               says one is missing.\n\
             - is the cited deed still the tip, or has a later take superseded it. A\n\
               citation that resolves and is stale is worse than one that fails, because\n\
               nothing complains.\n\
             - what else stands on anything that moved. `vissue_backlinks` on the\n\
               accession walks it, and the answer is the blast radius rather than one\n\
               issue's problem.\n\
             \n\
             Report the accession, which question it failed, and what waits on it. An\n\
             issue whose citations all hold gets one line saying so."
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vissue_core::config::{DEFAULT_PREFIX, Layout};

    fn server() -> (tempfile::TempDir, VissueServer) {
        let dir = tempfile::tempdir().expect("tempdir");
        let layout = Layout::new(dir.path(), DEFAULT_PREFIX);
        std::fs::create_dir_all(layout.projects_dir()).expect("dirs");
        let server = VissueServer::with_layout(layout);
        (dir, server)
    }

    fn text(message: &PromptMessage) -> &str {
        &message.content.as_text().expect("a text prompt").text
    }

    fn ordered(said: &str, verbs: &[&str]) {
        let at: Vec<usize> = verbs
            .iter()
            .map(|v| {
                said.find(v)
                    .unwrap_or_else(|| panic!("{v} missing: {said}"))
            })
            .collect();
        assert!(
            at.windows(2).all(|w| w[0] < w[1]),
            "{verbs:?} out of order: {said}"
        );
    }

    /// Every declared prompt renders, from the arguments it says it takes.
    #[tokio::test]
    async fn every_prompt_renders_from_what_it_declares() {
        let (_dir, server) = server();
        let declared = VissueServer::prompt_router().list_all();
        assert_eq!(declared.len(), 3, "{declared:?}");

        for prompt in &declared {
            assert!(
                prompt.description.as_ref().is_some_and(|d| !d.is_empty()),
                "{} carries no description",
                prompt.name
            );
            let arguments = prompt
                .arguments
                .as_ref()
                .unwrap_or_else(|| panic!("{} declares no arguments", prompt.name));
            assert!(!arguments.is_empty(), "{}", prompt.name);
        }

        let mut names: Vec<&str> = declared.iter().map(|p| p.name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, ["check_citations", "pack_a_slice", "pick_up_work"]);

        let packed = server
            .pack_a_slice_prompt(Parameters(SliceArgs {
                projects: Some("keys".into()),
                issues: None,
                out: Some("/tmp/bag".into()),
            }))
            .await
            .expect("renders");
        let said = text(&packed[0]);
        assert!(said.contains("projects keys"), "{said}");
        assert!(said.contains("/tmp/bag"), "{said}");
        ordered(
            said,
            &[
                "`vissue_satchel`",
                "`vissue_satchel_seal`",
                "`vissue_satchel_verify`",
            ],
        );

        let work = server
            .pick_up_work_prompt(Parameters(ReadyArgs {
                project: Some("keys".into()),
            }))
            .await
            .expect("renders");
        let said = text(&work[0]);
        assert!(said.contains("project keys"), "{said}");
        ordered(
            said,
            &["`vissue_ready`", "`vissue_recall`", "`vissue_deed`"],
        );

        let cites = server
            .check_citations_prompt(Parameters(IssueArgs {
                issue: "keys-ab12".into(),
            }))
            .await
            .expect("renders");
        let said = text(&cites[0]);
        assert!(said.contains("keys-ab12"), "{said}");
        ordered(said, &["`vissue_backlinks`"]);
    }

    /// A slice that names nothing is refused as a bad parameter.
    #[tokio::test]
    async fn a_slice_naming_nothing_is_refused() {
        let (_dir, server) = server();
        let err = server
            .pack_a_slice_prompt(Parameters(SliceArgs {
                projects: Some("  ".into()),
                issues: None,
                out: None,
            }))
            .await
            .expect_err("an empty slice rendered");
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    }

    /// Absent arguments render as their defaults, not as `None`.
    #[tokio::test]
    async fn the_defaults_are_in_the_text() {
        let (_dir, server) = server();
        let packed = server
            .pack_a_slice_prompt(Parameters(SliceArgs {
                projects: None,
                issues: Some("keys-ab12".into()),
                out: None,
            }))
            .await
            .expect("renders");
        let said = text(&packed[0]);
        assert!(said.contains("./satchel"), "{said}");
        assert!(said.contains("issues keys-ab12"), "{said}");

        let work = server
            .pick_up_work_prompt(Parameters(ReadyArgs { project: None }))
            .await
            .expect("renders");
        let said = text(&work[0]);
        assert!(!said.contains("Some(") && !said.contains("None"), "{said}");
    }
}
