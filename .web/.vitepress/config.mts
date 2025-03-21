import {defineConfig} from 'vitepress'

export default defineConfig({
    lang: 'en-US',
    title: "Passage",
    description: "Minecraft Server Transfer Router",

    head: [
        ["link", {rel: "icon", href: "/favicon.ico"}],
        ["meta", {name: "theme-color", content: "#ff6c32"}],
        ['meta', {property: 'og:type', content: 'website'}],
    ],

    cleanUrls: true,

    themeConfig: {
        logo: 'images/logo.png',

        nav: [
            {text: 'Home', link: '/'},
            {text: 'Examples', link: '/markdown-examples'},
            {text: 'Blog', link: 'https://scrayos.net'},
        ],

        sidebar: [
            {
                text: 'Examples',
                items: [
                    {text: 'Markdown Examples', link: '/markdown-examples'},
                    {text: 'Runtime API Examples', link: '/api-examples'}
                ]
            }
        ],

        socialLinks: [
            {icon: 'discord', link: 'https://discord.gg/xZ4wbuuKZf'},
            {icon: 'github', link: 'https://github.com/scrayosnet/passage'},
        ],

        footer: {
            message: 'Released under the MIT License',
            copyright: 'Copyright © 2025 Scrayos UG (haftungsbeschränkt)'
        },

        editLink: {
            pattern: 'https://github.com/scrayosnet/passage/edit/main/.web/:path',
            text: 'Edit this page on GitHub'
        },

        search: {
            provider: 'local'
        },

        externalLinkIcon: true,
    }
})
