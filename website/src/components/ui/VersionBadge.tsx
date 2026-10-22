import { useCopy } from '../../lib/hooks.ts';
import pkg from '../../../package.json';

// Set by release-website.yml to the commit the deploy was built from; unset
// locally, where there is no deployed build to name.
const buildSha = import.meta.env.VITE_WEBSITE_BUILD_SHA;

/**
 * Which build of the site you are looking at.
 *
 * Fixed to the corner rather than placed in the footer: a bug report is a
 * screenshot, and this has to be in every one of them without the reporter
 * scrolling to the bottom first. The site's version is read straight from
 * package.json, which is the file vump keeps in step with the website-v* tag,
 * so it cannot drift from the tag the way a separately-computed value could.
 */
export function VersionBadge() {
  const label = buildSha ? `v${pkg.version}-${buildSha.slice(0, 7)}` : `v${pkg.version}-local`;
  const { copied, copy } = useCopy(label);

  return (
    <button
      type="button"
      onClick={copy}
      title={buildSha ? `Built from commit ${buildSha}` : 'Local build'}
      className="text-faint/50 hover:text-muted fixed right-2 bottom-2 z-30 font-mono text-[10px] transition-colors"
    >
      {copied ? 'copied' : label}
    </button>
  );
}
