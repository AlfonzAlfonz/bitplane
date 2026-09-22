// @ts-check
// This runs in Node.js - Don't use client-side code here (browser APIs, JSX...)

import {themes as prismThemes} from 'prism-react-renderer';

const owner = 'AlfonzAlfonz';
const repo = 'bitplane';

/** @type {import('@docusaurus/types').Config} */
const config = {
  title: 'bitplane',
  tagline: 'Named sets of git worktrees that share one lifecycle',

  // Opts in to every Docusaurus v4 default early, including the rspack-based
  // faster build. One thing this changes today: MDX 1 compatibility is off, so
  // an admonition titles itself with a directive label — `:::note[Title]`, not
  // `:::note Title`. The second form silently renders as literal text.
  future: {
    v4: true,
  },

  url: `https://${owner.toLowerCase()}.github.io`,
  baseUrl: `/${repo}/`,
  organizationName: owner,
  projectName: repo,
  trailingSlash: false,

  // A page that links to nothing, or to something that moved, is a docs bug.
  // Every one of these fails the build, and therefore CI.
  onBrokenLinks: 'throw',
  onBrokenAnchors: 'throw',
  markdown: {
    hooks: {
      onBrokenMarkdownLinks: 'throw',
      onBrokenMarkdownImages: 'throw',
    },
  },

  i18n: {
    defaultLocale: 'en',
    locales: ['en'],
  },

  presets: [
    [
      'classic',
      /** @type {import('@docusaurus/preset-classic').Options} */
      ({
        docs: {
          sidebarPath: './sidebars.js',
          // The docs are the site; there is no marketing landing page.
          routeBasePath: '/',
          editUrl: `https://github.com/${owner}/${repo}/tree/main/website/`,
          // Three status admonitions on top of the theme's own set, one per
          // state a reference page can be in. Rendered by
          // `src/theme/Admonition/Type/Status.js`, which is where the colours
          // and icons live; a keyword listed here and missing there degrades
          // to `info` with a warning rather than failing the build.
          admonitions: {
            extendDefaults: true,
            keywords: ['implemented', 'in-progress', 'not-implemented'],
          },
        },
        blog: false,
        theme: {
          customCss: './src/css/custom.css',
        },
      }),
    ],
  ],

  themeConfig:
    /** @type {import('@docusaurus/preset-classic').ThemeConfig} */
    ({
      colorMode: {
        respectPrefersColorScheme: true,
      },
      navbar: {
        title: 'bitplane',
        items: [
          {
            type: 'docSidebar',
            sidebarId: 'docs',
            position: 'left',
            label: 'Docs',
          },
          {
            type: 'docSidebar',
            sidebarId: 'reference',
            position: 'left',
            label: 'Reference',
          },
          {
            href: `https://github.com/${owner}/${repo}`,
            label: 'GitHub',
            position: 'right',
          },
        ],
      },
      footer: {
        style: 'dark',
        links: [],
        copyright: `Copyright © ${new Date().getFullYear()} Denis Homolík. GPL-2.0-or-later.`,
      },
      prism: {
        theme: prismThemes.github,
        darkTheme: prismThemes.dracula,
      },
    }),
};

export default config;
