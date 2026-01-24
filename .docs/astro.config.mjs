// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

// https://astro.build/config
export default defineConfig({
	integrations: [
		starlight({
			title: 'Passage',
			social: [
                { icon: 'github', label: 'GitHub', href: 'https://github.com/scrayosnet/passage' },
                { icon: 'discord', label: 'Discord', href: 'https://discord.gg/xZ4wbuuKZf' }
            ],
			sidebar: [
				{
					label: 'Overview',
                    autogenerate: { directory: 'overview' },
				},
				{
					label: 'Setup',
					autogenerate: { directory: 'setup' },
				},
                {
                    label: 'Customization',
                    autogenerate: { directory: 'customization' },
                },
			],
		}),
	],
});
