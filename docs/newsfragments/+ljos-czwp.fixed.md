An idle `vissue serve` now stays quiet when a file is only touched.
It also stays quiet when macOS reports writes from before the server started.
A change notice names only the projects whose issues changed.
The comparison is on parsed issues, not bytes: an edit that parses the same sends no notice, and issues that only moved lines send one.
