Sway and other Wayland binds that summon the HUD now pass the
xdg-activation token through, so the overlay maps with keyboard focus.
The first bind (no HUD yet) and a `.desktop` launch consume the token
the same way. `--hide` with no HUD running exits 0 and does not start one.
