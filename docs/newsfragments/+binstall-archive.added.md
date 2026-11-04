`cargo binstall vissue-cli` fetches a prebuilt archive.
`vissue-mcp` and `vissue-hud` fetch that same archive.
The name is `vissue-v<version>-<target>.tar.gz`.
A `.sha256` file sits beside it.
Apple targets are aarch64-apple-darwin and x86_64-apple-darwin.
Linux targets are aarch64-unknown-linux-gnu and x86_64-unknown-linux-gnu.
A tag builds those archives only after CI on that commit is green.
