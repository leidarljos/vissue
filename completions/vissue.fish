# Print an optspec for argparse to handle cmd's options that are independent of any subcommand.
function __fish_vissue_global_optspecs
    string join \n root= prefix= no-route h/help V/version
end

function __fish_vissue_needs_command
    # Figure out if the current invocation already has a command.
    set -l cmd (commandline -opc)
    set -e cmd[1]
    argparse -s (__fish_vissue_global_optspecs) -- $cmd 2>/dev/null
    or return
    if set -q argv[1]
        # Also print the command, so this can be used to figure out what it is.
        echo $argv[1]
        return 1
    end
    return 0
end

function __fish_vissue_using_subcommand
    set -l cmd (__fish_vissue_needs_command)
    test -z "$cmd"
    and return 1
    contains -- $cmd[1] $argv
end

complete -c vissue -n "__fish_vissue_needs_command" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_needs_command" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_needs_command" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_needs_command" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_needs_command" -s V -l version -d 'Print version'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "create" -d 'Create an issue. Pass the body with --body or --body-file (`-` reads stdin); omit both to leave the body empty for a later edit'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "q" -d 'Quick capture: create and print only the id'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "list" -d 'List issues, sorted by priority then state then id'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "show" -d 'Show one issue: metadata, then the body'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "update" -d 'Update state, priority, or blocker edges'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "resolve" -d 'Pick one terminal after a sibling close'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "reject" -d 'Reject an issue, redirecting to an existing destination or a new replacement'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "ready" -d 'Actionable issues: TODO or STARTED with no open blocker'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "claim" -d 'Take an issue: move it to STARTED and stamp the claim'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "release" -d 'Drop every live claim held by one identity. State stays'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "vote" -d 'Cast this agent\'s vote on an issue, or show the tally with no `--for`'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "deed" -d 'Cite, drop, or list the deeds this issue\'s work produced'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "recall" -d 'The working set for an issue: its plan, its inputs\' deeds, and its own'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "consensus" -d 'Weigh an issue\'s ballots by who the group listens to (DeGroot)'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "note" -d 'Add a dated note to the top of an issue\'s logbook; state and claim untouched'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "append" -d 'Append a dated report to an issue\'s body'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "claims" -d 'Every live claim, oldest first: who holds what, and for how long'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "fold" -d 'Fold an inbox org file: each unstamped `* TODO <title>` heading becomes an issue, then the heading is stamped with the id and flipped to DONE in place. Already-stamped headings are skipped'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "agenda" -d 'Dated open work: deadlines and scheduled starts inside a horizon, overdue first'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "hygiene" -d 'Checklist for agents and CI: stalled claims plus corpus validation'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "whoami" -d 'Print the identity this tracker would record on a claim'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "waiting-on" -d 'Issues waiting on this one'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "body-excerpt" -d 'The first lines of an issue\'s file range'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "search" -d 'Substring search over ids, titles, properties, and bodies'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "children" -d 'Issues whose `:PARENT:` matches this id'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "ancestors" -d 'Blockers transitively required by this issue'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "impact" -d 'Issues transitively waiting on this issue'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "related" -d 'Explain bounded Org and lexical connections around an issue'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "stale" -d 'Open issues whose `:CREATED:` is older than N days'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "count" -d 'Print only the matching issue count'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "export" -d 'One JSON object per issue per line'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "tree" -d 'Children and blockers below an id'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "cycles" -d 'Cycles in the blocker graph'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "graph" -d 'The blocker and parent graph as Graphviz DOT'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "satchel" -d 'Pack a slice of the tracker so somebody else can open it'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "refile" -d 'Move an issue to another project\'s file'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "backlinks" -d 'Issues referring to this id'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "roadmap" -d 'A markdown roadmap of active and closed work'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "check" -d 'Validate the corpus. Exits non-zero on any error'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "normalize" -d 'Rewrite files onto the Org / ELPA / vissue property split'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "digest" -d 'A content digest of the corpus, for telling whether a copy is current'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "project" -d 'Run the projection this repository\'s vissue.toml declares: fold each board\'s inbox into its source, apply its claims file, rewrite its mirror. A source not on this machine is reported and skipped'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "mirror" -d 'Write a read-only projection of one or more projects to a file'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "events" -d 'Change events with a sequence above --since'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "ping" -d 'Append a manual event, waking pollers without editing an issue'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "wait" -d 'Block until the generation passes --last, or until an issue is terminal. Exits 2 on timeout'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "gen" -d 'Print the current generation counter'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "projects" -d 'List the projects found under the layout prefix'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "surface" -d 'This binary\'s own surface as JSON: every subcommand, its aliases, and its long flags. Hidden; the schema checks read it'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "identity" -d 'Print the resolved binary, root, and prefix'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "serve" -d 'Own the per-user Unix control socket'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "tui" -d 'Interactive board over ready, list, claims, agenda, and search'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "hud" -d 'Task board. Default execs `vissue-hud`. Home is the project list'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "completions" -d 'Write a shell completion script to stdout'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "man" -d 'Write the roff manual page to stdout'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "keys" -d 'Print the HUD key catalog, or check a keys.toml overlay'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c vissue -n "__fish_vissue_using_subcommand create" -s p -l project -d 'Project name. Auto-detected from .project-ctx.toml when omitted' -r
complete -c vissue -n "__fish_vissue_using_subcommand create" -l priority -d 'Priority cookie: A high, B mid, C low' -r
complete -c vissue -n "__fish_vissue_using_subcommand create" -s t -l type -d 'Type tag such as feature, bug, or task' -r
complete -c vissue -n "__fish_vissue_using_subcommand create" -l deadline -d 'Org deadline like `<2026-05-15 Fri>` or `[2026-05-15]`' -r
complete -c vissue -n "__fish_vissue_using_subcommand create" -l scheduled -d 'Org scheduled date like `<2026-05-01 Mon>`' -r
complete -c vissue -n "__fish_vissue_using_subcommand create" -l tags -d 'Comma- or colon-separated tags' -r
complete -c vissue -n "__fish_vissue_using_subcommand create" -l parent -d 'Parent id, which must already exist' -r
complete -c vissue -n "__fish_vissue_using_subcommand create" -l id -d 'Keep this id instead of minting one. Form is `{project}-` plus `0-9a-z`' -r
complete -c vissue -n "__fish_vissue_using_subcommand create" -l body -d 'Body text written under the heading' -r
complete -c vissue -n "__fish_vissue_using_subcommand create" -l body-file -d 'Read the body from a file; `-` reads stdin' -r
complete -c vissue -n "__fish_vissue_using_subcommand create" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand create" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand create" -s q -l quiet -d 'Print only the new id'
complete -c vissue -n "__fish_vissue_using_subcommand create" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand create" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand q" -s p -l project -d 'Project the issue belongs to; the current file\'s project when omitted' -r
complete -c vissue -n "__fish_vissue_using_subcommand q" -s t -l type -d 'Issue type, recorded as a tag: task, bug, epic, or any word' -r
complete -c vissue -n "__fish_vissue_using_subcommand q" -l parent -d 'Parent id, which must already exist' -r
complete -c vissue -n "__fish_vissue_using_subcommand q" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand q" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand q" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand q" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand list" -s p -l project -d 'Only this project; every project when omitted' -r
complete -c vissue -n "__fish_vissue_using_subcommand list" -s s -l state -d 'Filter by state: TODO, STARTED, BLOCKED, DONE, or CANCELLED' -r
complete -c vissue -n "__fish_vissue_using_subcommand list" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand list" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand list" -l json -d 'Emit JSON rows instead of text'
complete -c vissue -n "__fish_vissue_using_subcommand list" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand list" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand show" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand show" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand show" -l json -d 'Emit a JSON object instead of text'
complete -c vissue -n "__fish_vissue_using_subcommand show" -l org -d 'Emit the heading\'s org text in full, nothing else. Use this to write the issue out as the specification someone works from'
complete -c vissue -n "__fish_vissue_using_subcommand show" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand show" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand update" -s s -l state -d 'New state: TODO, STARTED, BLOCKED, DONE, or CANCELLED' -r
complete -c vissue -n "__fish_vissue_using_subcommand update" -l priority -d 'New priority: A, B, or C' -r
complete -c vissue -n "__fish_vissue_using_subcommand update" -l block -d 'Add a blocker edge' -r
complete -c vissue -n "__fish_vissue_using_subcommand update" -l unblock -d 'Remove a blocker edge' -r
complete -c vissue -n "__fish_vissue_using_subcommand update" -l if-state -d 'Refuse unless the heading is still this state' -r
complete -c vissue -n "__fish_vissue_using_subcommand update" -l if-gen -d 'Refuse unless the corpus generation is still this value' -r
complete -c vissue -n "__fish_vissue_using_subcommand update" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand update" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand update" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand update" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand resolve" -s s -l state -d 'The terminal state to pick: DONE or CANCELLED' -r
complete -c vissue -n "__fish_vissue_using_subcommand resolve" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand resolve" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand resolve" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand resolve" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand reject" -l to -d 'Existing destination issue' -r
complete -c vissue -n "__fish_vissue_using_subcommand reject" -s p -l project -d 'Project for a newly created replacement' -r
complete -c vissue -n "__fish_vissue_using_subcommand reject" -l reason -d 'Why this issue is rejected' -r
complete -c vissue -n "__fish_vissue_using_subcommand reject" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand reject" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand reject" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand reject" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand ready" -s p -l project -d 'Only this project; every project when omitted' -r
complete -c vissue -n "__fish_vissue_using_subcommand ready" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand ready" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand ready" -l json -d 'Emit JSON rows instead of text'
complete -c vissue -n "__fish_vissue_using_subcommand ready" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand ready" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand claim" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand claim" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand claim" -l force -d 'Take over a claim held by another identity'
complete -c vissue -n "__fish_vissue_using_subcommand claim" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand claim" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand release" -l holder -d 'Identity whose claims to drop' -r
complete -c vissue -n "__fish_vissue_using_subcommand release" -l older-than -d 'Only claims whose newest claim or note is older than this many days' -r
complete -c vissue -n "__fish_vissue_using_subcommand release" -l why -d 'Why this holder is being released; written on each ticket' -r
complete -c vissue -n "__fish_vissue_using_subcommand release" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand release" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand release" -l dry-run -d 'Print what would be released without writing'
complete -c vissue -n "__fish_vissue_using_subcommand release" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand release" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand vote" -l for -d 'What to vote for. Omit to read the tally without casting' -r
complete -c vissue -n "__fish_vissue_using_subcommand vote" -l used -d 'Deed accessions this ballot used, or `none`' -r
complete -c vissue -n "__fish_vissue_using_subcommand vote" -l confidence -d 'Probability in (0, 1] that the choice is the outcome' -r
complete -c vissue -n "__fish_vissue_using_subcommand vote" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand vote" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand vote" -l json -d 'The ballots as JSON rows of `agent`, `choice`, `stamp`; reads only'
complete -c vissue -n "__fish_vissue_using_subcommand vote" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand vote" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c vissue -n "__fish_vissue_using_subcommand deed" -l add -d 'A deed accession this issue produced. Repeatable' -r
complete -c vissue -n "__fish_vissue_using_subcommand deed" -l remove -d 'A citation to drop. Repeatable' -r
complete -c vissue -n "__fish_vissue_using_subcommand deed" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand deed" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand deed" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand deed" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c vissue -n "__fish_vissue_using_subcommand recall" -s d -l depth -d 'Hops of the blocker walk. One is enough when the deeds carry their own sources, which `deedar trail` walks' -r
complete -c vissue -n "__fish_vissue_using_subcommand recall" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand recall" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand recall" -l deeds-only -d 'Print only the deed accessions, one per line'
complete -c vissue -n "__fish_vissue_using_subcommand recall" -l excerpts -d 'Include a capped excerpt of each input\'s heading'
complete -c vissue -n "__fish_vissue_using_subcommand recall" -l json -d 'Emit a JSON object instead of text'
complete -c vissue -n "__fish_vissue_using_subcommand recall" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand recall" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c vissue -n "__fish_vissue_using_subcommand consensus" -l trust -d 'Trust rows laid over `[consensus.trust]`: `[[from, to, weight], ...]`' -r
complete -c vissue -n "__fish_vissue_using_subcommand consensus" -l susceptibility-of -d 'Per-agent susceptibility laid over `[consensus.susceptibility_of]`: `{"agent": s}` with s in [0, 1]; 0 holds the agent to its ballot' -r
complete -c vissue -n "__fish_vissue_using_subcommand consensus" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand consensus" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand consensus" -l children -d 'Roll up over this issue\'s children instead of reading its own ballots'
complete -c vissue -n "__fish_vissue_using_subcommand consensus" -l gate -d 'Exit non-zero when there is nothing settled to act on'
complete -c vissue -n "__fish_vissue_using_subcommand consensus" -l json -d 'Emit a JSON object instead of text'
complete -c vissue -n "__fish_vissue_using_subcommand consensus" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand consensus" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c vissue -n "__fish_vissue_using_subcommand note" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand note" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand note" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand note" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand append" -l text -d 'The text to append' -r
complete -c vissue -n "__fish_vissue_using_subcommand append" -l file -d 'Read the text from a file; `-` reads stdin' -r
complete -c vissue -n "__fish_vissue_using_subcommand append" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand append" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand append" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand append" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c vissue -n "__fish_vissue_using_subcommand claims" -l by -d 'Only claims held by this identity' -r
complete -c vissue -n "__fish_vissue_using_subcommand claims" -s p -l project -d 'Only claims in this project' -r
complete -c vissue -n "__fish_vissue_using_subcommand claims" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand claims" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand claims" -l json -d 'Machine-readable output'
complete -c vissue -n "__fish_vissue_using_subcommand claims" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand claims" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand fold" -s p -l project -d 'Project the folded issues are created in. Auto-detected from .project-ctx.toml when omitted' -r
complete -c vissue -n "__fish_vissue_using_subcommand fold" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand fold" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand fold" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand fold" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand agenda" -s d -l days -d 'Days ahead to include' -r
complete -c vissue -n "__fish_vissue_using_subcommand agenda" -s p -l project -d 'Only this project; every project when omitted' -r
complete -c vissue -n "__fish_vissue_using_subcommand agenda" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand agenda" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand agenda" -l json -d 'Emit a JSON array instead of text'
complete -c vissue -n "__fish_vissue_using_subcommand agenda" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand agenda" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand hygiene" -l stale-days -d 'Days a claim may be held before it counts as stale' -r
complete -c vissue -n "__fish_vissue_using_subcommand hygiene" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand hygiene" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand hygiene" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand hygiene" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand whoami" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand whoami" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand whoami" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand whoami" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand waiting-on" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand waiting-on" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand waiting-on" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand waiting-on" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand body-excerpt" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand body-excerpt" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand body-excerpt" -l json -d 'Emit a JSON object instead of text'
complete -c vissue -n "__fish_vissue_using_subcommand body-excerpt" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand body-excerpt" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand search" -s n -l limit -d 'Most hits to print' -r
complete -c vissue -n "__fish_vissue_using_subcommand search" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand search" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand search" -l json -d 'Emit a JSON array instead of text'
complete -c vissue -n "__fish_vissue_using_subcommand search" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand search" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand children" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand children" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand children" -l json -d 'Emit a JSON array instead of text'
complete -c vissue -n "__fish_vissue_using_subcommand children" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand children" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand ancestors" -s d -l depth -d 'How many blocker hops to follow' -r
complete -c vissue -n "__fish_vissue_using_subcommand ancestors" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand ancestors" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand ancestors" -l json -d 'Emit a JSON array instead of text'
complete -c vissue -n "__fish_vissue_using_subcommand ancestors" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand ancestors" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand impact" -s d -l depth -d 'How many waiting hops to follow' -r
complete -c vissue -n "__fish_vissue_using_subcommand impact" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand impact" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand impact" -l json -d 'Emit a JSON array instead of text'
complete -c vissue -n "__fish_vissue_using_subcommand impact" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand impact" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand related" -s d -l depth -d 'How many hops to follow' -r
complete -c vissue -n "__fish_vissue_using_subcommand related" -s n -l limit -d 'Most connections to print' -r
complete -c vissue -n "__fish_vissue_using_subcommand related" -l format -d 'text or org; org emits links to the source headings' -r
complete -c vissue -n "__fish_vissue_using_subcommand related" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand related" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand related" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand related" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand stale" -s d -l days -d 'Age in days past which an open issue counts as stale' -r
complete -c vissue -n "__fish_vissue_using_subcommand stale" -s p -l project -d 'Only this project; every project when omitted' -r
complete -c vissue -n "__fish_vissue_using_subcommand stale" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand stale" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand stale" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand stale" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand count" -s p -l project -d 'Only this project; every project when omitted' -r
complete -c vissue -n "__fish_vissue_using_subcommand count" -s s -l state -d 'Count only issues in this state: TODO, STARTED, BLOCKED, DONE, or CANCELLED' -r
complete -c vissue -n "__fish_vissue_using_subcommand count" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand count" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand count" -s r -l ready -d 'Count only actionable issues'
complete -c vissue -n "__fish_vissue_using_subcommand count" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand count" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand export" -s p -l project -d 'Only this project; every project when omitted' -r
complete -c vissue -n "__fish_vissue_using_subcommand export" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand export" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand export" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand export" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand tree" -s f -l format -d 'ascii or dot' -r
complete -c vissue -n "__fish_vissue_using_subcommand tree" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand tree" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand tree" -l json -d 'Emit the tree as JSON instead of ascii or dot'
complete -c vissue -n "__fish_vissue_using_subcommand tree" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand tree" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand cycles" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand cycles" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand cycles" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand cycles" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand graph" -s p -l project -d 'Only this project; every project when omitted' -r
complete -c vissue -n "__fish_vissue_using_subcommand graph" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand graph" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand graph" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand graph" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand satchel" -l out -d 'Where to write the satchel' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand satchel" -l project -d 'Take these projects whole. Repeatable' -r
complete -c vissue -n "__fish_vissue_using_subcommand satchel" -l issue -d 'Take these issues, and whatever they stand on. Repeatable' -r
complete -c vissue -n "__fish_vissue_using_subcommand satchel" -l seal -d 'Re-manifest a satchel over everything now in its payload' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand satchel" -l verify -d 'Check a satchel that arrived' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand satchel" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand satchel" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand satchel" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand satchel" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c vissue -n "__fish_vissue_using_subcommand refile" -l to -d 'Target project' -r
complete -c vissue -n "__fish_vissue_using_subcommand refile" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand refile" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand refile" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand refile" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand backlinks" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand backlinks" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand backlinks" -l json -d 'Emit a JSON array instead of text'
complete -c vissue -n "__fish_vissue_using_subcommand backlinks" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand backlinks" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand roadmap" -s p -l project -d 'Only this project; every project when omitted' -r
complete -c vissue -n "__fish_vissue_using_subcommand roadmap" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand roadmap" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand roadmap" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand roadmap" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand check" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand check" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand check" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand check" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand normalize" -s p -l project -d 'Only this project; every project when omitted' -r
complete -c vissue -n "__fish_vissue_using_subcommand normalize" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand normalize" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand normalize" -l dry-run -d 'Print what would change without writing'
complete -c vissue -n "__fish_vissue_using_subcommand normalize" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand normalize" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand digest" -s p -l project -d 'Project to include; repeat for several. Omit for every project' -r
complete -c vissue -n "__fish_vissue_using_subcommand digest" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand digest" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand digest" -l json -d 'Emit a JSON object instead of text'
complete -c vissue -n "__fish_vissue_using_subcommand digest" -s q -l quiet -d 'Print only the combined digest'
complete -c vissue -n "__fish_vissue_using_subcommand digest" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand digest" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand project" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand project" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand project" -l check -d 'Compare each mirror\'s stamp against its source instead of writing. Exits 0 when every reachable mirror is fresh, 1 when one is stale'
complete -c vissue -n "__fish_vissue_using_subcommand project" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand project" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand mirror" -s p -l project -d 'Project to include; repeat for several. Omit for every project' -r
complete -c vissue -n "__fish_vissue_using_subcommand mirror" -s o -l out -d 'Destination file; `-` writes to standard output' -r
complete -c vissue -n "__fish_vissue_using_subcommand mirror" -l check -d 'Compare an existing mirror\'s stamp against the tracker instead of writing. Exits 0 when fresh, 1 when stale' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand mirror" -s f -l format -d 'org or markdown' -r
complete -c vissue -n "__fish_vissue_using_subcommand mirror" -s s -l state -d 'Include only this state' -r
complete -c vissue -n "__fish_vissue_using_subcommand mirror" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand mirror" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand mirror" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand mirror" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand events" -l since -d 'Only events newer than this sequence' -r
complete -c vissue -n "__fish_vissue_using_subcommand events" -s n -l limit -d 'Maximum events returned' -r
complete -c vissue -n "__fish_vissue_using_subcommand events" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand events" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand events" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand events" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand ping" -l detail -d 'A line recorded with the event' -r
complete -c vissue -n "__fish_vissue_using_subcommand ping" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand ping" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand ping" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand ping" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand wait" -l last -d 'The generation already seen; returns once the counter passes it' -r
complete -c vissue -n "__fish_vissue_using_subcommand wait" -l id -d 'Issue to watch when --until-terminal is set' -r
complete -c vissue -n "__fish_vissue_using_subcommand wait" -l poll-ms -d 'How often to look, in milliseconds' -r
complete -c vissue -n "__fish_vissue_using_subcommand wait" -l timeout-ms -d 'Give up after this many milliseconds; exit 2' -r
complete -c vissue -n "__fish_vissue_using_subcommand wait" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand wait" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand wait" -l until-terminal -d 'Block until the issue is DONE or CANCELLED'
complete -c vissue -n "__fish_vissue_using_subcommand wait" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand wait" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand gen" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand gen" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand gen" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand gen" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand projects" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand projects" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand projects" -l json -d 'Emit a JSON array instead of one name per line'
complete -c vissue -n "__fish_vissue_using_subcommand projects" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand projects" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand surface" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand surface" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand surface" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand surface" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand identity" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand identity" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand identity" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand identity" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand serve; and not __fish_seen_subcommand_from stop restart status help" -s s -l socket -d 'Control socket path. Falls back to VISSUE_CONTROL_SOCKET, then $XDG_RUNTIME_DIR/vissue/control.sock, then ~/.vissue/run/control.sock' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand serve; and not __fish_seen_subcommand_from stop restart status help" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand serve; and not __fish_seen_subcommand_from stop restart status help" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand serve; and not __fish_seen_subcommand_from stop restart status help" -s d -l detach -d 'Detach after the socket accepts. The child is placed in its own process group (not a new session) and can still receive SIGHUP from the parent terminal'
complete -c vissue -n "__fish_vissue_using_subcommand serve; and not __fish_seen_subcommand_from stop restart status help" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand serve; and not __fish_seen_subcommand_from stop restart status help" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c vissue -n "__fish_vissue_using_subcommand serve; and not __fish_seen_subcommand_from stop restart status help" -f -a "stop" -d 'Signal the owner (SIGTERM, then SIGKILL) and wait'
complete -c vissue -n "__fish_vissue_using_subcommand serve; and not __fish_seen_subcommand_from stop restart status help" -f -a "restart" -d 'Stop, then start detached'
complete -c vissue -n "__fish_vissue_using_subcommand serve; and not __fish_seen_subcommand_from stop restart status help" -f -a "status" -d 'Print a live/pid/socket snapshot. Exit 0 if live, 1 otherwise'
complete -c vissue -n "__fish_vissue_using_subcommand serve; and not __fish_seen_subcommand_from stop restart status help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c vissue -n "__fish_vissue_using_subcommand serve; and __fish_seen_subcommand_from stop" -s s -l socket -d 'Control socket path. Falls back to VISSUE_CONTROL_SOCKET, then $XDG_RUNTIME_DIR/vissue/control.sock, then ~/.vissue/run/control.sock' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand serve; and __fish_seen_subcommand_from stop" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand serve; and __fish_seen_subcommand_from stop" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand serve; and __fish_seen_subcommand_from stop" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand serve; and __fish_seen_subcommand_from stop" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand serve; and __fish_seen_subcommand_from restart" -s s -l socket -d 'Control socket path. Falls back to VISSUE_CONTROL_SOCKET, then $XDG_RUNTIME_DIR/vissue/control.sock, then ~/.vissue/run/control.sock' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand serve; and __fish_seen_subcommand_from restart" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand serve; and __fish_seen_subcommand_from restart" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand serve; and __fish_seen_subcommand_from restart" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand serve; and __fish_seen_subcommand_from restart" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand serve; and __fish_seen_subcommand_from status" -s s -l socket -d 'Control socket path. Falls back to VISSUE_CONTROL_SOCKET, then $XDG_RUNTIME_DIR/vissue/control.sock, then ~/.vissue/run/control.sock' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand serve; and __fish_seen_subcommand_from status" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand serve; and __fish_seen_subcommand_from status" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand serve; and __fish_seen_subcommand_from status" -l json -d 'Machine-readable object'
complete -c vissue -n "__fish_vissue_using_subcommand serve; and __fish_seen_subcommand_from status" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand serve; and __fish_seen_subcommand_from status" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand serve; and __fish_seen_subcommand_from help" -f -a "stop" -d 'Signal the owner (SIGTERM, then SIGKILL) and wait'
complete -c vissue -n "__fish_vissue_using_subcommand serve; and __fish_seen_subcommand_from help" -f -a "restart" -d 'Stop, then start detached'
complete -c vissue -n "__fish_vissue_using_subcommand serve; and __fish_seen_subcommand_from help" -f -a "status" -d 'Print a live/pid/socket snapshot. Exit 0 if live, 1 otherwise'
complete -c vissue -n "__fish_vissue_using_subcommand serve; and __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c vissue -n "__fish_vissue_using_subcommand tui" -s s -l socket -d 'Control socket path. Falls back to VISSUE_CONTROL_SOCKET, then $XDG_RUNTIME_DIR/vissue/control.sock, then ~/.vissue/run/control.sock' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand tui" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand tui" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand tui" -l offline -d 'Never attach, never spawn serve; CatalogService plus generation poll'
complete -c vissue -n "__fish_vissue_using_subcommand tui" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand tui" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c vissue -n "__fish_vissue_using_subcommand hud" -l mode -d 'ready, list (all), claims, stale, or new. Used by `--rofi`' -r
complete -c vissue -n "__fish_vissue_using_subcommand hud" -s s -l socket -d 'Control socket path. Falls back to VISSUE_CONTROL_SOCKET, then $XDG_RUNTIME_DIR/vissue/control.sock, then ~/.vissue/run/control.sock' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand hud" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand hud" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand hud" -l offline -d 'Never attach, never spawn serve'
complete -c vissue -n "__fish_vissue_using_subcommand hud" -l toggle -d 'Show or hide a running board, or dismiss a live rofi picker'
complete -c vissue -n "__fish_vissue_using_subcommand hud" -l show -d 'Show a running board'
complete -c vissue -n "__fish_vissue_using_subcommand hud" -l hide -d 'Hide a running board, or dismiss a live rofi picker'
complete -c vissue -n "__fish_vissue_using_subcommand hud" -l install-desktop -d 'Write a user-local .desktop launcher and Sway overlay include'
complete -c vissue -n "__fish_vissue_using_subcommand hud" -l iced -d 'Use the iced board. Default when `--rofi` is absent'
complete -c vissue -n "__fish_vissue_using_subcommand hud" -l rofi -d 'Use the rofi picker instead of the iced board'
complete -c vissue -n "__fish_vissue_using_subcommand hud" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand hud" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c vissue -n "__fish_vissue_using_subcommand completions" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand completions" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand completions" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand completions" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c vissue -n "__fish_vissue_using_subcommand man" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand man" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand man" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand man" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand keys" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand keys" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand keys" -l check -d 'Load the overlay and exit 1 on conflict'
complete -c vissue -n "__fish_vissue_using_subcommand keys" -l occupancy -d 'Print taken chords'
complete -c vissue -n "__fish_vissue_using_subcommand keys" -l no-route -d 'Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and keep every verb on the process default layout'
complete -c vissue -n "__fish_vissue_using_subcommand keys" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "create" -d 'Create an issue. Pass the body with --body or --body-file (`-` reads stdin); omit both to leave the body empty for a later edit'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "q" -d 'Quick capture: create and print only the id'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "list" -d 'List issues, sorted by priority then state then id'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "show" -d 'Show one issue: metadata, then the body'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "update" -d 'Update state, priority, or blocker edges'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "resolve" -d 'Pick one terminal after a sibling close'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "reject" -d 'Reject an issue, redirecting to an existing destination or a new replacement'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "ready" -d 'Actionable issues: TODO or STARTED with no open blocker'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "claim" -d 'Take an issue: move it to STARTED and stamp the claim'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "release" -d 'Drop every live claim held by one identity. State stays'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "vote" -d 'Cast this agent\'s vote on an issue, or show the tally with no `--for`'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "deed" -d 'Cite, drop, or list the deeds this issue\'s work produced'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "recall" -d 'The working set for an issue: its plan, its inputs\' deeds, and its own'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "consensus" -d 'Weigh an issue\'s ballots by who the group listens to (DeGroot)'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "note" -d 'Add a dated note to the top of an issue\'s logbook; state and claim untouched'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "append" -d 'Append a dated report to an issue\'s body'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "claims" -d 'Every live claim, oldest first: who holds what, and for how long'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "fold" -d 'Fold an inbox org file: each unstamped `* TODO <title>` heading becomes an issue, then the heading is stamped with the id and flipped to DONE in place. Already-stamped headings are skipped'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "agenda" -d 'Dated open work: deadlines and scheduled starts inside a horizon, overdue first'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "hygiene" -d 'Checklist for agents and CI: stalled claims plus corpus validation'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "whoami" -d 'Print the identity this tracker would record on a claim'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "waiting-on" -d 'Issues waiting on this one'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "body-excerpt" -d 'The first lines of an issue\'s file range'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "search" -d 'Substring search over ids, titles, properties, and bodies'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "children" -d 'Issues whose `:PARENT:` matches this id'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "ancestors" -d 'Blockers transitively required by this issue'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "impact" -d 'Issues transitively waiting on this issue'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "related" -d 'Explain bounded Org and lexical connections around an issue'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "stale" -d 'Open issues whose `:CREATED:` is older than N days'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "count" -d 'Print only the matching issue count'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "export" -d 'One JSON object per issue per line'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "tree" -d 'Children and blockers below an id'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "cycles" -d 'Cycles in the blocker graph'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "graph" -d 'The blocker and parent graph as Graphviz DOT'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "satchel" -d 'Pack a slice of the tracker so somebody else can open it'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "refile" -d 'Move an issue to another project\'s file'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "backlinks" -d 'Issues referring to this id'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "roadmap" -d 'A markdown roadmap of active and closed work'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "check" -d 'Validate the corpus. Exits non-zero on any error'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "normalize" -d 'Rewrite files onto the Org / ELPA / vissue property split'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "digest" -d 'A content digest of the corpus, for telling whether a copy is current'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "project" -d 'Run the projection this repository\'s vissue.toml declares: fold each board\'s inbox into its source, apply its claims file, rewrite its mirror. A source not on this machine is reported and skipped'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "mirror" -d 'Write a read-only projection of one or more projects to a file'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "events" -d 'Change events with a sequence above --since'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "ping" -d 'Append a manual event, waking pollers without editing an issue'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "wait" -d 'Block until the generation passes --last, or until an issue is terminal. Exits 2 on timeout'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "gen" -d 'Print the current generation counter'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "projects" -d 'List the projects found under the layout prefix'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "surface" -d 'This binary\'s own surface as JSON: every subcommand, its aliases, and its long flags. Hidden; the schema checks read it'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "identity" -d 'Print the resolved binary, root, and prefix'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "serve" -d 'Own the per-user Unix control socket'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "tui" -d 'Interactive board over ready, list, claims, agenda, and search'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "hud" -d 'Task board. Default execs `vissue-hud`. Home is the project list'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "completions" -d 'Write a shell completion script to stdout'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "man" -d 'Write the roff manual page to stdout'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "keys" -d 'Print the HUD key catalog, or check a keys.toml overlay'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update resolve reject ready claim release vote deed recall consensus note append claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph satchel refile backlinks roadmap check normalize digest project mirror events ping wait gen projects surface identity serve tui hud completions man keys help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c vissue -n "__fish_vissue_using_subcommand help; and __fish_seen_subcommand_from serve" -f -a "stop" -d 'Signal the owner (SIGTERM, then SIGKILL) and wait'
complete -c vissue -n "__fish_vissue_using_subcommand help; and __fish_seen_subcommand_from serve" -f -a "restart" -d 'Stop, then start detached'
complete -c vissue -n "__fish_vissue_using_subcommand help; and __fish_seen_subcommand_from serve" -f -a "status" -d 'Print a live/pid/socket snapshot. Exit 0 if live, 1 otherwise'
