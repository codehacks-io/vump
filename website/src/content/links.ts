import release from './release.json';

const REPO = 'https://github.com/codehacks-io/vump';

/** The published vump release this build of the site describes. */
export const VUMP_VERSION: string = release.version;

export const links = {
  repo: REPO,
  readme: `${REPO}#readme`,
  releases: `${REPO}/releases`,
  /** The newest stable release, whatever it is when the visitor clicks. */
  latestRelease: `${REPO}/releases/latest`,
  /** The exact release this build states. */
  release: `${REPO}/releases/tag/v${VUMP_VERSION}`,
  action: `${REPO}/tree/main/.github/actions/check`,
} as const;
