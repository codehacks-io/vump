export const CI_WORKFLOW = `on:
  push:
    tags: ['v*']

jobs:
  release:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: codehacks-io/vump/.github/actions/check@main
        with:
          version: \${{ github.ref_name }}

      - run: cargo build --release`;

export const MULTI_PROJECT_CONFIG = `[[project]]
name = "api"
files = ["services/api/Cargo.toml", "Cargo.lock"]

[[project]]
name = "web"
files = ["apps/web/package.json"]`;

export const CONFIG = `files = ["VERSION", "ui/package.json"]

[git]
through = "tag"
commit_message = "chore: bump version to v{new_version}"
tag_pattern = "v{new_version}"`;
