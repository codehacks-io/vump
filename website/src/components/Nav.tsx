import { links, VUMP_VERSION } from '../content/links.ts';
import { cn } from '../lib/cn.ts';
import { useScrolled } from '../lib/hooks.ts';
import { GitHubIcon, Mark } from './ui/icons.tsx';
import { ThemeToggle } from './ui/ThemeToggle.tsx';

const SECTIONS = [
  { href: '#features', label: 'What it does' },
  { href: '#files', label: 'Files' },
  { href: '#ci', label: 'CI' },
  { href: '#start', label: 'Quick start' },
];

/**
 * The release the site describes, resolved at build time.
 *
 * Asking GitHub for it from the visitor's browser would put an unauthenticated
 * API call on every page load — rate-limited to 60 an hour per address, so the
 * chip would simply vanish for anyone behind a busy NAT — and would make the
 * page say different things at different moments. The version is a declared
 * input instead: see scripts/vump-version.mjs.
 */
function ReleaseChip() {
  return (
    <a
      href={links.release}
      target="_blank"
      rel="noreferrer noopener"
      className="border-line text-muted hover:border-line-strong hover:text-ink hidden items-center gap-2 rounded-full border px-3 py-1 font-mono text-[11px] transition-colors sm:inline-flex"
    >
      <span className="bg-signal h-1.5 w-1.5 rounded-full" aria-hidden />v{VUMP_VERSION}
    </a>
  );
}

export function Nav() {
  const scrolled = useScrolled();

  return (
    <header className="fixed inset-x-0 top-0 z-50">
      <div
        className={cn(
          'border-b transition-colors duration-300',
          scrolled ? 'border-line bg-ground/75 backdrop-blur-xl' : 'border-transparent',
        )}
      >
        <div className="mx-auto flex h-16 w-full max-w-[78rem] items-center justify-between gap-6 px-6 md:px-10">
          <a href="#top" className="text-ink flex items-center gap-2.5">
            <Mark className="h-6 w-6" />
            <span className="text-[15px] font-semibold tracking-[-0.02em]">vump</span>
          </a>

          <nav aria-label="Sections" className="hidden items-center gap-7 lg:flex">
            {SECTIONS.map((section) => (
              <a
                key={section.href}
                href={section.href}
                className="text-muted hover:text-ink text-[13.5px] transition-colors"
              >
                {section.label}
              </a>
            ))}
          </nav>

          <div className="flex items-center gap-2.5">
            <ReleaseChip />
            <a
              href={links.repo}
              target="_blank"
              rel="noreferrer noopener"
              aria-label="vump on GitHub"
              className="border-line text-muted hover:border-line-strong hover:text-ink inline-flex h-9 w-9 items-center justify-center rounded-full border transition-colors"
            >
              <GitHubIcon />
            </a>
            <ThemeToggle />
          </div>
        </div>
      </div>
    </header>
  );
}
