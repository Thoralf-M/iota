// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

import { themes } from "prism-react-renderer";
import path from "path";
import math from "remark-math";
import katex from "rehype-katex";
import codeImport from "remark-code-import";

require("dotenv").config();

const jargonConfig = require('./config/jargon.js');

/** @type {import('@docusaurus/types').Config} */
const config = {
  title: "IOTA Documentation",
  tagline:
    "IOTA is a next-generation smart contract platform with high throughput, low latency, and an asset-oriented programming model powered by Move",
  favicon: "/icons/favicon.ico",

  // Set the production url of your site here
  url: "https://docs.iota.org",
  // Set the /<baseUrl>/ pathname under which your site is served
  // For GitHub pages deployment, it is often '/<projectName>/'
  baseUrl: "/",
  customFields: {
    amplitudeKey: process.env.AMPLITUDE_KEY,
  },

  // TODO: Revert the changes when the docs are ready
  onBrokenLinks: "throw",
  onBrokenMarkdownLinks: "throw",
  onBrokenAnchors: "throw",

  // Even if you don't use internationalization, you can use this field to set
  // useful metadata like html lang. For example, if your site is Chinese, you
  // may want to replace "en" with "zh-Hans".
  /*  i18n: {
    defaultLocale: "en",
    locales: [
      "en",
      "el",
      "fr",
      "ko",
      "tr",
      "vi",
      "zh-CN",
      "zh-TW",
    ],
  },*/
  markdown: {
    format: "detect",
    mermaid: true,
  },
  plugins: [
    [
      "@graphql-markdown/docusaurus",
      /** @type {import('@graphql-markdown/types').ConfigOptions} */
      {
        id:'mainnet',
        schema: "https://raw.githubusercontent.com/iotaledger/iota/refs/heads/mainnet/crates/iota-graphql-rpc/schema.graphql",
        rootPath: "../content", // docs will be generated under rootPath/baseURL
        baseURL: "references/iota-api/iota-graphql/reference/",
        loaders: {
          UrlLoader: {
            module: "@graphql-tools/url-loader",
          }
        },
      },
    ],
    [
      "@graphql-markdown/docusaurus",
      /** @type {import('@graphql-markdown/types').ConfigOptions} */
      {
        id:'testnet',
        schema: "https://raw.githubusercontent.com/iotaledger/iota/refs/heads/testnet/crates/iota-graphql-rpc/schema.graphql",
        rootPath: "../content", // docs will be generated under rootPath/baseURL
        baseURL: "references/iota-api/iota-graphql/reference/testnet/",
        loaders: {
          UrlLoader: {
            module: "@graphql-tools/url-loader",
          }
        },
      },
    ],
    [
      "@graphql-markdown/docusaurus",
      /** @type {import('@graphql-markdown/types').ConfigOptions} */
      {
        id:'devnet',
        schema: "https://raw.githubusercontent.com/iotaledger/iota/refs/heads/devnet/crates/iota-graphql-rpc/schema.graphql",
        rootPath: "../content", // docs will be generated under rootPath/baseURL
        baseURL: "references/iota-api/iota-graphql/reference/devnet/",
        loaders: {
          UrlLoader: {
            module: "@graphql-tools/url-loader",
          }
        },
      },
    ],
    async function myPlugin(context, options) {
      return {
        name: "docusaurus-tailwindcss",
        configurePostCss(postcssOptions) {
          // Appends TailwindCSS and AutoPrefixer.
          postcssOptions.plugins.push(require("tailwindcss"));
          postcssOptions.plugins.push(require("autoprefixer"));
          return postcssOptions;
        },
      };
    },
    path.resolve(__dirname, `./src/plugins/descriptions`),
    [
      'docusaurus-plugin-typedoc',
      // Options
      {
        tsconfig: '../../sdk/typescript/tsconfig.json',
        entryPoints: [
          "../../sdk/typescript/src/bcs",
          "../../sdk/typescript/src/client",
          "../../sdk/typescript/src/cryptography",
          "../../sdk/typescript/src/faucet",
          "../../sdk/typescript/src/graphql",
          "../../sdk/typescript/src/keypairs/ed25519",
          "../../sdk/typescript/src/keypairs/secp256k1",
          "../../sdk/typescript/src/keypairs/secp256r1",
          "../../sdk/typescript/src/multisig",
          "../../sdk/typescript/src/transactions",
          "../../sdk/typescript/src/utils",
          "../../sdk/typescript/src/verify"
        ],
        plugin: ["typedoc-plugin-markdown"],
        out: "../generated-docs/ts-sdk",
        githubPages: false,
        readme: "none",
        hideGenerator: true,
        sort: ["source-order"],
        excludeInternal: true,
        excludePrivate: true,
        disableSources: true,
        hideBreadcrumbs: true,
        intentionallyNotExported: [],
      },
    ],
    [
      '@docusaurus/plugin-client-redirects',
      {
        createRedirects(existingPath) {
          const redirects = [
            {
              from: '/references/ts-sdk',
              to: '/ts-sdk',
            },
          ];
          let paths = [];
          for (const redirect of redirects) {
            if (existingPath.startsWith(redirect.to)) {
              paths.push(existingPath.replace(redirect.to, redirect.from));
            }
          }
          return paths.length > 0 ? paths : undefined;
        },
      },
    ],
    'plugin-image-zoom',
    [
      'docusaurus-plugin-openapi-docs',
      {
        id: 'openapi',
        docsPluginId: 'classic',
        config: {
          coreApiV2: {
            specPath:
              'https://raw.githubusercontent.com/iotaledger/wasp/refs/heads/develop/clients/apiclient/api/openapi.yaml',
            outputDir: 
              '../content/iota-evm/references/openapi',
            sidebarOptions: {
              groupPathsBy: 'tag',
            }
          }
        }
      }
    ]
  ],
  presets: [
    [
      "classic",
      /** @type {import('@docusaurus/preset-classic').Options} */
      ({
        docs: {
          path: "../content",
          routeBasePath: "/",
          sidebarPath: require.resolve("./sidebars.js"),
          docItemComponent: "@theme/ApiItem", // Derived from docusaurus-theme-openapi
          docRootComponent: "@theme/DocRoot", // add @theme/DocRoot
          async sidebarItemsGenerator({
            isCategoryIndex: defaultCategoryIndexMatcher, // The default matcher implementation, given below
            defaultSidebarItemsGenerator,
            ...args
          }) {
            return defaultSidebarItemsGenerator({
              ...args,
              isCategoryIndex(doc) {
                if(doc.fileName === 'index' && doc.directories.includes('ts-sdk'))
                  return true;
                // No doc will be automatically picked as category index
                return false;
              },
            });
          },
          // the double docs below is a fix for having the path set to ../content
          editUrl: "https://github.com/iotaledger/iota/tree/develop/docs/docs",
          onInlineTags: "throw",
          
          /*disableVersioning: true,
          lastVersion: "current",
          versions: {
            current: {
              label: "Latest",
              path: "/",
            },
          },
          onlyIncludeVersions: [
            "current",
            "1.0.0",
          ],*/
          remarkPlugins: [
            [math,{singleDollarTextMath:false}],
            [
              require("@docusaurus/remark-plugin-npm2yarn"),
              { sync: true, converters: ["yarn", "pnpm"] },
            ],
            [codeImport, { rootDir: path.resolve(__dirname, `../../`) }],
          ],
          rehypePlugins: [
            katex,
            [require('rehype-jargon'), { jargon: jargonConfig}]
          ],
        },
        theme: {
          customCss: [
            require.resolve("./src/css/fonts.css"),
            require.resolve("./src/css/custom.css"),
          ],
        },
      }),
    ],
  ],
  stylesheets: [
    {
      href: "https://fonts.googleapis.com/css2?family=Inter:wght@400;500;700&display=swap",
      type: "text/css",
    },
    {
      href: "https://cdn.jsdelivr.net/npm/katex@0.13.24/dist/katex.min.css",
      type: "text/css",
      integrity:
        "sha384-odtC+0UGzzFL/6PNoE8rX/SPcQDXBJ+uRepguP4QkPCm2LBxH3FA3y+fKSiJ+AmM",
      crossorigin: "anonymous",
    },
    {
      href: "https://cdnjs.cloudflare.com/ajax/libs/font-awesome/6.5.1/css/all.min.css",
      type: "text/css",
    },
  ],
  themes: [
    '@docusaurus/theme-mermaid',
    '@saucelabs/theme-github-codeblock', 
    '@docusaurus/theme-live-codeblock',
    'docusaurus-theme-openapi-docs',
  ],
  themeConfig:
    /** @type {import('@docusaurus/preset-classic').ThemeConfig} */
    ({
      algolia: {
        apiKey: '24b141ea7e65db2181463e44dbe564a5',
        appId: '9PMBZGRP3B',
        indexName: 'iota',
      },
      image: "img/iota-doc-og.png",
      docs: {
        sidebar: {
          autoCollapseCategories: false,
        },
      },
      colorMode: {
        defaultMode: "dark",
      },
      announcementBar: {
        id: "integrate_your_exchange",
        content:
          '<a target="_blank" rel="noopener noreferrer" href="/developer/exchange-integration/">Integrate your exchange</a>. If you supported Stardust, please make sure to also <a target="_blank" rel="noopener noreferrer" href="/developer/stardust/exchanges"> migrate from Stardust</a>.',
        isCloseable: false,
        backgroundColor: "#0101ff",
        textColor: "#FFFFFF",
      },
      navbar: {
        title: "",
        logo: {
          alt: "IOTA Docs Logo",
          src: "/logo/iota-logo.svg",
        },
        items: [
          {
            label: "About IOTA",
            to: "about-iota",
          },
          {
            label: "Developers",
            to: "developer",
          },
          {
            label: "Node Operators",
            to: "operator",
          },
          {
            label: "References",
            to: "references",
          },
          {
            label: "TS SDK",
            to: "ts-sdk/typescript/",
          },
          {
            label: "IOTA Identity",
            to: "iota-identity",
          },
          {
            label: "IOTA EVM",
            to: "iota-evm",
          },
          {
            type: 'custom-WalletConnectButton',
            position: 'right',
          },
        ],
      },
      footer: {
        style: "dark",
        logo: {
          alt: "IOTA Wiki Logo",
          src: "/logo/iota-logo.svg",
        },
        copyright: `Copyright © ${new Date().getFullYear()} <a href='https://www.iota.org/'>IOTA Stiftung</a>, licensed under <a href="https://github.com/iotaledger/iota/blob/develop/docs/site/LICENSE">CC BY 4.0</a>. 
                    The documentation on this website is adapted from the <a href='https://docs.sui.io/'>SUI Documentation</a>, © 2024 by <a href='https://sui.io/'>SUI Foundation</a>, licensed under <a href="https://github.com/MystenLabs/sui/blob/main/docs/site/LICENSE">CC BY 4.0</a>.`,
      },
      socials: [
        'https://www.youtube.com/c/iotafoundation',
        'https://www.github.com/iotaledger/',
        'https://discord.gg/iota-builders',
        'https://discord.iota.org/',
        'https://www.twitter.com/iota/',
        'https://www.reddit.com/r/iota/',
        'https://www.linkedin.com/company/iotafoundation/',
        'https://www.instagram.com/iotafoundation/',
      ],
      prism: {
        theme: themes.vsLight,
        darkTheme: themes.vsDark,
        additionalLanguages: ["rust", "typescript", "solidity", "move"],
      },
      imageZoom: {
        selector: '.markdown img',
        // Optional medium-zoom options
        // see: https://www.npmjs.com/package/medium-zoom#options
        options: {
          background: 'rgba(0, 0, 0, 0.6)',
        },
      }
    }),
};

export default config;
