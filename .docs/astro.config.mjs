// @ts-check
import {defineConfig} from 'astro/config';
import starlight from '@astrojs/starlight';
import mermaid from "astro-mermaid";
import starlightLinksValidator from 'starlight-links-validator'
import sitemap from '@astrojs/sitemap';
import starlightLlmsTxt from "starlight-llms-txt";

// https://astro.build/config
export default defineConfig({
    site: 'https://passage.scrayos.net/',
    integrations: [starlight({
        title: 'Passage',
        social: [
            {icon: 'github', label: 'GitHub', href: 'https://github.com/scrayosnet/passage'},
            {icon: 'discord', label: 'Discord', href: 'https://discord.gg/xZ4wbuuKZf'}
        ],
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
                autogenerate: {directory: 'reference'},
            },
        ],
        plugins: [starlightLinksValidator(), starlightLlmsTxt()],
    }), mermaid({
        theme: 'forest',
        autoTheme: true
    }), sitemap()],
});
