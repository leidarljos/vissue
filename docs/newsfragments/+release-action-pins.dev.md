The release workflow pins each action to the commit of a tag or branch.
dtolnay/rust-toolchain is pinned to the master commit that tag v1 names.
Top-level permissions stay read-only.
The release job writes the GitHub release and its attestations.
The publish job grants id-token so the called workflow can mint a token.
Every job that GitHub allows to take a time limit has one.
