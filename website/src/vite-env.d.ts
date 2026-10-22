/// <reference types="vite/client" />

interface ImportMetaEnv {
  /**
   * Short commit the site was built and deployed from, set by
   * release-website.yml. Unset for local builds, where there is no deploy to
   * point at.
   */
  readonly VITE_WEBSITE_BUILD_SHA?: string;
}
