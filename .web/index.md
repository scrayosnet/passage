---
# https://vitepress.dev/reference/default-theme-home-page
layout: home

hero:
    name: "Passage"
    text: "Minecraft Server Transfer Router"
    tagline: "Connect your Minecraft network to the world without any limits to scaling, security or infrastructure in a few seconds!"
    image:
        src: 'logo.png'
        alt: 'Passage'
    actions:
        -   theme: brand
            text: Getting Started
            link: /getting-started
        -   theme: alt
            text: Discord
            link: https://discord.gg/xZ4wbuuKZf
            target: '_blank'
        -   theme: alt
            text: GitHub
            link: https://github.com/scrayosnet/passage
            target: '_blank'

features:
    -   icon: '✨'
        title: Stateless and proxy-less
        details: Passage holds no connection and can therefore be restarted and replicated at will.
        linkText: Explore the architecture
        link: /architecture
    -   icon: '🚀'
        title: Fast and reliable
        details: Passage is faster and more reliable than any proxy software and needs less than 1MB.
        linkText: Compare with other proxies
        link: /comparison
    -   icon: '🔐'
        title: Secure
        details: Passage uses modern cryptography and makes sure that only valid players are let through.
        linkText: Read about the details
        link: /authentication-and-encryption
    -   icon: '🔋'
        title: Fast to set up
        details: Passage is ready to connect your network with your players in seconds. Batteries included!
        linkText: Install it
        link: /installation
    -   icon: '🖌️'
        title: Customizable
        details: All algorithms, messages and adapters can be customized and tailored to your needs.
        linkText: Configure and customize your setup
        link: /customization
    -   icon: '📊'
        title: Scales infinitely
        details: Passage can handle an unlimited number of players without any additional effort.
        linkText: Scale your network
        link: /scaling
---

