export const links = {
    base: 'https://passage.scrayos.net',
    discord: 'https://discord.gg/xZ4wbuuKZf',
    github: 'https://github.com/scrayosnet/passage',
}

export const commitRef = process.env.CF_PAGES_COMMIT_SHA?.slice(0, 8) || 'dev'
