# Print an optspec for argparse to handle cmd's options that are independent of any subcommand.
function __fish_vissue_global_optspecs
    string join \n root= prefix= h/help V/version
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
complete -c vissue -n "__fish_vissue_needs_command" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_needs_command" -s V -l version -d 'Print version'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "create" -d 'Create an issue. Pass the body with --body or --body-file (`-` reads stdin); omit both to leave the body empty for a later edit'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "q" -d 'Quick capture: create and print only the id'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "list" -d 'List issues, sorted by priority then state then id'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "show" -d 'Show one issue\'s metadata and file range'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "update" -d 'Update state, priority, or blocker edges'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "ready" -d 'Actionable issues: TODO or STARTED with no open blocker'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "claim" -d 'Take an issue: move it to STARTED and stamp the claim'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "note" -d 'Add a dated note to the top of an issue\'s logbook; state and claim untouched'
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
complete -c vissue -n "__fish_vissue_needs_command" -f -a "refile" -d 'Move an issue to another project\'s file'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "backlinks" -d 'Issues referring to this id'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "roadmap" -d 'A markdown roadmap of active and closed work'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "check" -d 'Validate the corpus. Exits non-zero on any error'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "digest" -d 'A content digest of the corpus, for telling whether a copy is current'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "mirror" -d 'Write a read-only projection of one or more projects to a file'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "events" -d 'Change events with a sequence above --since'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "ping" -d 'Append a manual event, waking pollers without editing an issue'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "wait" -d 'Block until the generation passes --last. Exits 2 on timeout'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "gen" -d 'Print the current generation counter'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "projects" -d 'List the projects found under the layout prefix'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "identity" -d 'Print the resolved binary, root, and prefix'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "completions" -d 'Write a shell completion script to stdout'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "man" -d 'Write the roff manual page to stdout'
complete -c vissue -n "__fish_vissue_needs_command" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c vissue -n "__fish_vissue_using_subcommand create" -s p -l project -d 'Project name. Auto-detected from .project-ctx.toml when omitted' -r
complete -c vissue -n "__fish_vissue_using_subcommand create" -l priority -d 'Priority cookie: A high, B mid, C low' -r
complete -c vissue -n "__fish_vissue_using_subcommand create" -s t -l type -d 'Type tag such as feature, bug, or task' -r
complete -c vissue -n "__fish_vissue_using_subcommand create" -l deadline -d 'Org deadline like `<2026-05-15 Fri>` or `[2026-05-15]`' -r
complete -c vissue -n "__fish_vissue_using_subcommand create" -l scheduled -d 'Org scheduled date like `<2026-05-01 Mon>`' -r
complete -c vissue -n "__fish_vissue_using_subcommand create" -l tags -d 'Comma- or colon-separated tags' -r
complete -c vissue -n "__fish_vissue_using_subcommand create" -l parent -d 'Parent id, which must already exist' -r
complete -c vissue -n "__fish_vissue_using_subcommand create" -l body -d 'Body text written under the heading' -r
complete -c vissue -n "__fish_vissue_using_subcommand create" -l body-file -d 'Read the body from a file; `-` reads stdin' -r
complete -c vissue -n "__fish_vissue_using_subcommand create" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand create" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand create" -s q -l quiet -d 'Print only the new id'
complete -c vissue -n "__fish_vissue_using_subcommand create" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand q" -s p -l project -r
complete -c vissue -n "__fish_vissue_using_subcommand q" -s t -l type -r
complete -c vissue -n "__fish_vissue_using_subcommand q" -l parent -r
complete -c vissue -n "__fish_vissue_using_subcommand q" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand q" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand q" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand list" -s p -l project -r
complete -c vissue -n "__fish_vissue_using_subcommand list" -s s -l state -d 'Filter by state: TODO, STARTED, BLOCKED, DONE, or CANCELLED' -r
complete -c vissue -n "__fish_vissue_using_subcommand list" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand list" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand list" -l json -d 'Emit JSON rows instead of text'
complete -c vissue -n "__fish_vissue_using_subcommand list" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand show" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand show" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand show" -l json -d 'Emit a JSON object instead of text'
complete -c vissue -n "__fish_vissue_using_subcommand show" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand update" -s s -l state -r
complete -c vissue -n "__fish_vissue_using_subcommand update" -l priority -r
complete -c vissue -n "__fish_vissue_using_subcommand update" -l block -d 'Add a blocker edge' -r
complete -c vissue -n "__fish_vissue_using_subcommand update" -l unblock -d 'Remove a blocker edge' -r
complete -c vissue -n "__fish_vissue_using_subcommand update" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand update" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand update" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand ready" -s p -l project -r
complete -c vissue -n "__fish_vissue_using_subcommand ready" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand ready" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand ready" -l json
complete -c vissue -n "__fish_vissue_using_subcommand ready" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand claim" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand claim" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand claim" -l force -d 'Take over a claim held by another identity'
complete -c vissue -n "__fish_vissue_using_subcommand claim" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand note" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand note" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand note" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand claims" -l by -d 'Only claims held by this identity' -r
complete -c vissue -n "__fish_vissue_using_subcommand claims" -s p -l project -d 'Only claims in this project' -r
complete -c vissue -n "__fish_vissue_using_subcommand claims" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand claims" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand claims" -l json -d 'Machine-readable output'
complete -c vissue -n "__fish_vissue_using_subcommand claims" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand fold" -s p -l project -d 'Project the folded issues are created in. Auto-detected from .project-ctx.toml when omitted' -r
complete -c vissue -n "__fish_vissue_using_subcommand fold" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand fold" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand fold" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand agenda" -s d -l days -d 'Days ahead to include' -r
complete -c vissue -n "__fish_vissue_using_subcommand agenda" -s p -l project -r
complete -c vissue -n "__fish_vissue_using_subcommand agenda" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand agenda" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand agenda" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand hygiene" -l stale-days -d 'Days a claim may be held before it counts as stale' -r
complete -c vissue -n "__fish_vissue_using_subcommand hygiene" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand hygiene" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand hygiene" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand whoami" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand whoami" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand whoami" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand waiting-on" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand waiting-on" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand waiting-on" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand body-excerpt" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand body-excerpt" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand body-excerpt" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand search" -s n -l limit -r
complete -c vissue -n "__fish_vissue_using_subcommand search" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand search" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand search" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand children" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand children" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand children" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand ancestors" -s d -l depth -r
complete -c vissue -n "__fish_vissue_using_subcommand ancestors" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand ancestors" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand ancestors" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand impact" -s d -l depth -r
complete -c vissue -n "__fish_vissue_using_subcommand impact" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand impact" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand impact" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand related" -s d -l depth -r
complete -c vissue -n "__fish_vissue_using_subcommand related" -s n -l limit -r
complete -c vissue -n "__fish_vissue_using_subcommand related" -l format -d 'text or org; org emits links to the source headings' -r
complete -c vissue -n "__fish_vissue_using_subcommand related" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand related" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand related" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand stale" -s d -l days -r
complete -c vissue -n "__fish_vissue_using_subcommand stale" -s p -l project -r
complete -c vissue -n "__fish_vissue_using_subcommand stale" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand stale" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand stale" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand count" -s p -l project -r
complete -c vissue -n "__fish_vissue_using_subcommand count" -s s -l state -r
complete -c vissue -n "__fish_vissue_using_subcommand count" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand count" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand count" -s r -l ready -d 'Count only actionable issues'
complete -c vissue -n "__fish_vissue_using_subcommand count" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand export" -s p -l project -r
complete -c vissue -n "__fish_vissue_using_subcommand export" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand export" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand export" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand tree" -s f -l format -d 'ascii or dot' -r
complete -c vissue -n "__fish_vissue_using_subcommand tree" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand tree" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand tree" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand cycles" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand cycles" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand cycles" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand graph" -s p -l project -r
complete -c vissue -n "__fish_vissue_using_subcommand graph" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand graph" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand graph" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand refile" -l to -d 'Target project' -r
complete -c vissue -n "__fish_vissue_using_subcommand refile" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand refile" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand refile" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand backlinks" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand backlinks" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand backlinks" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand roadmap" -s p -l project -r
complete -c vissue -n "__fish_vissue_using_subcommand roadmap" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand roadmap" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand roadmap" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand check" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand check" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand check" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand digest" -s p -l project -d 'Project to include; repeat for several. Omit for every project' -r
complete -c vissue -n "__fish_vissue_using_subcommand digest" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand digest" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand digest" -l json -d 'Emit a JSON object instead of text'
complete -c vissue -n "__fish_vissue_using_subcommand digest" -s q -l quiet -d 'Print only the combined digest'
complete -c vissue -n "__fish_vissue_using_subcommand digest" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand mirror" -s p -l project -d 'Project to include; repeat for several. Omit for every project' -r
complete -c vissue -n "__fish_vissue_using_subcommand mirror" -s o -l out -d 'Destination file; `-` writes to standard output' -r
complete -c vissue -n "__fish_vissue_using_subcommand mirror" -l check -d 'Compare an existing mirror\'s stamp against the tracker instead of writing. Exits 0 when fresh, 1 when stale' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand mirror" -s f -l format -d 'org or markdown' -r
complete -c vissue -n "__fish_vissue_using_subcommand mirror" -s s -l state -d 'Include only this state' -r
complete -c vissue -n "__fish_vissue_using_subcommand mirror" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand mirror" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand mirror" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand events" -l since -d 'Only events newer than this sequence' -r
complete -c vissue -n "__fish_vissue_using_subcommand events" -s n -l limit -d 'Maximum events returned' -r
complete -c vissue -n "__fish_vissue_using_subcommand events" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand events" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand events" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand ping" -l detail -r
complete -c vissue -n "__fish_vissue_using_subcommand ping" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand ping" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand ping" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand wait" -l last -r
complete -c vissue -n "__fish_vissue_using_subcommand wait" -l poll-ms -r
complete -c vissue -n "__fish_vissue_using_subcommand wait" -l timeout-ms -r
complete -c vissue -n "__fish_vissue_using_subcommand wait" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand wait" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand wait" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand gen" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand gen" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand gen" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand projects" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand projects" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand projects" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand identity" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand identity" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand identity" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand completions" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand completions" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand completions" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c vissue -n "__fish_vissue_using_subcommand man" -l root -d 'Tracker root. Falls back to ISSUE_ROOT, VISSUE_ROOT, then the current directory' -r -F
complete -c vissue -n "__fish_vissue_using_subcommand man" -l prefix -d 'Directory under the root holding one subdirectory per project. Falls back to VISSUE_PREFIX, then `prefix` in vissue.toml, then `Software`' -r
complete -c vissue -n "__fish_vissue_using_subcommand man" -s h -l help -d 'Print help'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "create" -d 'Create an issue. Pass the body with --body or --body-file (`-` reads stdin); omit both to leave the body empty for a later edit'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "q" -d 'Quick capture: create and print only the id'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "list" -d 'List issues, sorted by priority then state then id'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "show" -d 'Show one issue\'s metadata and file range'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "update" -d 'Update state, priority, or blocker edges'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "ready" -d 'Actionable issues: TODO or STARTED with no open blocker'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "claim" -d 'Take an issue: move it to STARTED and stamp the claim'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "note" -d 'Add a dated note to the top of an issue\'s logbook; state and claim untouched'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "claims" -d 'Every live claim, oldest first: who holds what, and for how long'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "fold" -d 'Fold an inbox org file: each unstamped `* TODO <title>` heading becomes an issue, then the heading is stamped with the id and flipped to DONE in place. Already-stamped headings are skipped'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "agenda" -d 'Dated open work: deadlines and scheduled starts inside a horizon, overdue first'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "hygiene" -d 'Checklist for agents and CI: stalled claims plus corpus validation'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "whoami" -d 'Print the identity this tracker would record on a claim'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "waiting-on" -d 'Issues waiting on this one'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "body-excerpt" -d 'The first lines of an issue\'s file range'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "search" -d 'Substring search over ids, titles, properties, and bodies'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "children" -d 'Issues whose `:PARENT:` matches this id'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "ancestors" -d 'Blockers transitively required by this issue'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "impact" -d 'Issues transitively waiting on this issue'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "related" -d 'Explain bounded Org and lexical connections around an issue'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "stale" -d 'Open issues whose `:CREATED:` is older than N days'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "count" -d 'Print only the matching issue count'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "export" -d 'One JSON object per issue per line'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "tree" -d 'Children and blockers below an id'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "cycles" -d 'Cycles in the blocker graph'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "graph" -d 'The blocker and parent graph as Graphviz DOT'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "refile" -d 'Move an issue to another project\'s file'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "backlinks" -d 'Issues referring to this id'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "roadmap" -d 'A markdown roadmap of active and closed work'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "check" -d 'Validate the corpus. Exits non-zero on any error'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "digest" -d 'A content digest of the corpus, for telling whether a copy is current'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "mirror" -d 'Write a read-only projection of one or more projects to a file'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "events" -d 'Change events with a sequence above --since'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "ping" -d 'Append a manual event, waking pollers without editing an issue'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "wait" -d 'Block until the generation passes --last. Exits 2 on timeout'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "gen" -d 'Print the current generation counter'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "projects" -d 'List the projects found under the layout prefix'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "identity" -d 'Print the resolved binary, root, and prefix'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "completions" -d 'Write a shell completion script to stdout'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "man" -d 'Write the roff manual page to stdout'
complete -c vissue -n "__fish_vissue_using_subcommand help; and not __fish_seen_subcommand_from create q list show update ready claim note claims fold agenda hygiene whoami waiting-on body-excerpt search children ancestors impact related stale count export tree cycles graph refile backlinks roadmap check digest mirror events ping wait gen projects identity completions man help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
