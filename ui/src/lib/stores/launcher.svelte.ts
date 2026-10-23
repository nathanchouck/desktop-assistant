/**
 * Whether the launcher is on screen. The window is hidden, not closed, so
 * the page keeps running: anything that repeats (status polling, clocks)
 * should only run while it's visible.
 */
class LauncherVisibility {
  visible = $state(true);
}

export const launcher = new LauncherVisibility();
