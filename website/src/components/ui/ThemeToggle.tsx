import { toggleTheme } from '../../lib/theme.ts';
import { MoonIcon, SunIcon } from './icons.tsx';

/**
 * Both icons are rendered and one is hidden by the theme attribute in CSS.
 * Choosing between them in JavaScript would mean the prerendered HTML picks a
 * theme the visitor may not be using, and hydration would swap it under them.
 */
export function ThemeToggle() {
  return (
    <button
      type="button"
      onClick={toggleTheme}
      aria-label="Switch between the light and dark theme"
      className="border-line text-muted hover:border-line-strong hover:text-ink inline-flex h-9 w-9 items-center justify-center rounded-full border transition-colors"
    >
      <SunIcon className="hidden dark:block" />
      <MoonIcon className="block dark:hidden" />
    </button>
  );
}
