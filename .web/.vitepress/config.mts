import {defineConfig} from 'vitepress'
import {commitRef, links} from "../util/config";

export default defineConfig({
    lang: 'en-US',
    title: "Passage",
    description: "Minecraft Server Transfer Router",

    sitemap: {
        hostname: links.base
    },

    lastUpdated: true,

    head: [
        ['link', {rel: 'icon', type: 'image/png', href: '/favicon.png'}],
        ["meta", {name: "theme-color", content: "#ff6c32"}],
        // open graph
        ['meta', {property: 'og:type', content: 'website'}],
        ['meta', {property: 'og:site_name', content: 'Passage'}],
        ['meta', {property: 'og:locale', content: 'en_US'}],
        ['meta', {property: 'og:image', content: `${links.base}/opengraph.png`}],
        ['meta', {property: 'og:url', content: links.base}],
        // twitter
        ['meta', {property: 'twitter:card', content: 'summary_large_image'}],
        ['meta', {property: 'twitter:site', content: '@ScrayosNET'}],
        ['meta', {property: 'twitter:image', content: `${links.base}/opengraph.png`}],

    ],

    transformHead: async (context) => {
        let title = context.title
        let description = context.description
        if (context.page == "index.md") {
            title = 'Passage – Minecraft Server Transfer Router'
            description = 'Passage is a fast, secure, and stateless Minecraft Server Transfer Router that connects your network effortlessly—scaling infinitely without the hassles of traditional proxies.'
        }

        return [
            // open graph
            ['meta', {property: 'og:title', content: title}],
            ['meta', {property: 'og:description', content: description}],
            // twitter
            ['meta', {property: 'twitter:title', content: title}],
            ['meta', {property: 'twitter:description', content: description}],
        ]
    },

    cleanUrls: true,

    themeConfig: {
        logo: 'logo.png',

        nav: [
            {text: 'Home', link: '/'},
            {text: 'Documentation', link: '/docs'},
            {text: 'Blog', link: 'https://scrayos.net'},
        ],

        sidebar: [
            {
                text: 'Overview',
                items: [
                    {text: 'Introduction', link: '/docs/index'},
                    {text: 'Architecture', link: '/docs/architecture'},
                    {text: 'Scaling', link: '/docs/scaling'},
                    {text: 'Authentication', link: '/docs/authentication-and-encryption'},
                    {text: 'Comparison', link: '/docs/comparison'},
                ]
            },
            {
                text: 'Setup',
                items: [
                    {text: 'Getting Started', link: '/docs/getting-started'},
                    {text: 'Installation', link: '/docs/installation'}
                ]
            },
            {
                text: 'Customization',
                items: [
                    {text: 'Overview', link: '/docs/customization'},
                ]
            }
        ],

        socialLinks: [
            {icon: 'discord', link: links.discord},
            {icon: 'github', link: links.github},
        ],

        footer: {
            message: `Released under the MIT License (${commitRef})`,
            copyright: 'Copyright © 2025 Scrayos UG (haftungsbeschränkt)'
        },

        editLink: {
            pattern: `${links.github}/edit/main/.web/:path`,
            text: 'Edit this page on GitHub'
        },

        search: {
            provider: 'local'
        },

        externalLinkIcon: true,
    }
})
