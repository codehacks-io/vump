/**
 * The theme lives on the document element, not in React state.
 *
 * A script in index.html resolves it before first paint, which is what keeps
 * the page from flashing the wrong one. Mirroring that into state would give
 * the server-rendered HTML a theme it cannot know and make hydration disagree
 * with what is already on screen; reading the attribute at the moment of the
 * click cannot drift from it.
 */

const KEY = 'vump-theme';

export function toggleTheme() {
  const root = document.documentElement;
  const next = root.dataset['theme'] === 'light' ? 'dark' : 'light';
  root.dataset['theme'] = next;

  try {
    localStorage.setItem(KEY, next);
  } catch {
    // A browser refusing storage is not a reason to refuse the theme.
  }
}
