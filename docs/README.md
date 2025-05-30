## Overview

This directory contains the assets required to build and update the [IOTA documentation](https://docs.iota.org). The directory is split between `content` and `site`. To run the site locally, open the root directory in a terminal or console. Use a package manager to install the required modules:

```shell
pnpm install
```

Still in root, the site should be built locally:

```shell
pnpm iota-docs build
```

This is necessary in the first instance with a freshly cloned repo as it will [download the OpenRPC specifications](/docs/site/src/utils/getopenrpcspecs.js) which are required to deploy the site.

Next, use the following command to deploy a development preview of the site to `localhost:3000`:

```shell
pnpm iota-docs dev
```

> If you're running the site locally and getting an error saying that you don't have `open-rpc` specs, run `pnpm build` first. It will prepare the files and fix the issue.

The deployment watches for updates to files in the `content` directory (and site source files), updating the UI to match any saves you make.

Once you've finished making changes, you should again run `pnpm build`. This builds the static site and places the files in `site\build`. This is important to run before submitting your changes for review, because a build will fail on errors like bad internal links, displaying the cause of the error to the console. The development preview ignores such errors to provide a more agile environment.

IOTA Foundation is not able to provide support for building the documentation site locally. If you run into trouble, consult the [Docusaurus](https://docusaurus.io/) documentation.

## Pull requests

IOTA uses Vercel to host its documentation site. Vercel builds a preview of the documentation for every pull request submitted to the IOTA repo. You can find a link to this preview in the PR comment section from the Vercel bot. Click the **Visit Preview** link for the **iota-core** project to verify your changes behave as you expect.

If you'd like to view the Vercel preview before your changes are ready for review, then [mark your PR as a draft](https://github.blog/2019-02-14-introducing-draft-pull-requests/).

## Contributing

IOTA is for the community. Contribute for the benefit of all.

- [Docs contributing guidelines](https://docs.iota.org/references/contribute/contribution-process)
- [Repo contributing guidelines](https://docs.iota.org/references/contribute/contribute-to-iota-repos)
- [Style guide](https://docs.iota.org/references/contribute/style-guide)
- [Code of conduct](https://docs.iota.org/references/contribute/code-of-conduct)

## License

The IOTA Documentation is distributed under the [LICENSE](CC BY 4.0 license).
