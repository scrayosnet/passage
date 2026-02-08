// @ts-check
import {defineConfig} from 'astro/config';
import starlight from '@astrojs/starlight';
import mermaid from "astro-mermaid";
import starlightLinksValidator from 'starlight-links-validator'
import sitemap from '@astrojs/sitemap';
import starlightLlmsTxt from "starlight-llms-txt";

import tailwindcss from '@tailwindcss/vite';

// https://astro.build/config
export default defineConfig({
    site: 'https://passage.scrayos.net/',
    integrations: [starlight({
        title: 'Passage',
        social: [
            {icon: 'github', label: 'GitHub', href: 'https://github.com/scrayosnet/passage'},
            {icon: 'discord', label: 'Discord', href: 'https://discord.gg/xZ4wbuuKZf'}
        ],
        logo: {
            src: './src/assets/logo-navbar.svg',
        },
        sidebar: [
            {
                label: 'Overview',
                autogenerate: {directory: 'overview'},
            },
            {
                label: 'Setup',
                autogenerate: {directory: 'setup'},
            },
            {
                label: 'Customization',
                autogenerate: {directory: 'customization'},
            },
            {
                label: 'Advanced',
                autogenerate: {directory: 'advanced'},
            },
            {
                label: 'Reference',
                autogenerate: {directory: 'reference', collapsed: true},
                collapsed: true,
            },
        ],
        lastUpdated: true,
        customCss: [
            './src/styles/global.css',
        ],
        editLink: {
            baseUrl: 'https://github.com/scrayosnet/passage/edit/main/.docs/',
        },
        plugins: [starlightLinksValidator(), starlightLlmsTxt()],
    }), mermaid({
        theme: 'forest',
        autoTheme: true
    }), sitemap()],

    vite: {
        plugins: [tailwindcss()],
    },
});
