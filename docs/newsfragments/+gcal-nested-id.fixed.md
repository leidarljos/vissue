`check` no longer treats a nested-project issue id as an org-gcal event.

A vissue id is `{project}-{suffix}`. When the project lives in a nested
directory the project name contains `/` (`Infra/terra-6cx2`), which is
not the org-gcal `<event>/<calendar>` form. A real org-gcal id is still
an error, and that heading is still left as Org around the issues.
