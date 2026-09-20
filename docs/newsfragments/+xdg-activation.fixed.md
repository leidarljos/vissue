Sway and other Wayland binds that summon the HUD now pass the
xdg-activation token through, so the overlay maps with keyboard focus.
`--hide` with no HUD running exits 0 and does not start one.
