![The official Logo of passage](.github/images/logo.png "passage")

![A visual badge for the latest release](https://img.shields.io/github/v/release/scrayosnet/passage "Latest Release")
![A visual badge for the workflow status](https://img.shields.io/github/actions/workflow/status/scrayosnet/passage/docker.yml "Workflow Status")
![A visual badge for the dependency status](https://img.shields.io/librariesio/github/scrayosnet/passage "Dependencies")
![A visual badge for the Docker image size](https://ghcr-badge.egpl.dev/scrayosnet/passage/size "Image Size")
![A visual badge for the license](https://img.shields.io/github/license/scrayosnet/passage "License")

Passage is a Minecraft network [transfer][transfer-packet] router to send connecting players to their corresponding
servers and act as an entrypoint to the network. While traditional Minecraft networks relied on proxies like
[BungeeCord][bungeecord-docs], [Waterfall][waterfall-docs] or [Velocity][velocity-docs], Passage is only an entrypoint
and initial router and drops the connection to the player right after the routing, improving performance and reliability
in the process.

This is possible through the [transfer packet][transfer-packet] of the official Minecraft: Java Edition
[protocol]. Passage validates connecting players, handles authentication, resource pack installation and status pings
and then redirects the players to any dynamic backend server. Since the [transfer packet][transfer-packet] was only
added in Minecraft [1.20.5][minecraft-1-20-5], Passage can only handle Minecraft clients starting from this version.

## Motivation

Despite the universal success and reliability of conventional proxies like [Velocity][velocity-docs],
[Waterfall][waterfall-docs] and [BungeeCord][bungeecord-docs], the general concept of a proxy that transcribes all
packages brings a lot of problems with it. Since those problems are inherent to the concept itself, this cannot be
solved by patching the existing proxies, but instead a new kind of network has to be created.

Traditional proxies need to transcode all Minecraft packets and adjust the contents to be consistent for the player's
connection. Switching servers is simulated by switching worlds. This means that proxies need to be updated for each
iteration of the Minecraft protocol and new versions are only supported after said update. Additionally, this
transcribing limits the possible throughput and requires additional computing resources as well as a slightly increased
latency.

Another problem with proxies is the scalability. Running a single instance is easy and will suffice for most networks,
but once a network has enough players, the territory of multi-proxy setups needs to be explored. This comes with its own
set of problems and complexity. But even if one proxy is able to host enough players, there are still risks of using
only a single proxy. Having no replication creates a single point of failure and all connections need to be dropped, if
the proxy is restarted, disconnecting all players in the process.

Finally, conventional proxies have problems to preserve the Mojang chat signature chain because they need to transcode
all messages and inject their own messages into the chat. While there are a few rumors around chat signing, there are
[many good reasons][chat-signing-explanation] to support chat signing. While it is not entirely impossible to use chat
signing with proxies, there is just a lot of inherent complexity and lots of things that can go wrong. With a transfer
based approach, chat signing is supported out-of-the-box.

You can find a [detailed comparison][passage-comparison] with more aspects on our website.

## Feature Highlights

* Connect your Minecraft network without any proxy or permanent connection with excellent reliability and performance, ready within seconds.
* Route your players to dynamic servers using Service discovery and an algorithm of your preference.
* Stay always online and drop no player connections during your maintenance or partial outages.
* Authenticate and validate your players with custom logic, to prevent any load from your backend servers.
* Prepare your players even before they reach your server, making sure the desired resource packs are loaded.
* Be ready for the next Minecraft version, without any necessity to update Passage beforehand.
* Support Mojang chat signing for a better protection of your players and advanced chat features.

Visit [our website][passage-website] to get a full overview over Passage's features.

## Getting Started

> [!WARNING]
> Passage is under active development and may experience breaking changes until the stable version 1.0.0 is released.
> After that version, breaking changes will be performed in adherence to [Semantic Versioning][semver-docs].

Install your own instance of Passage within seconds with our [Getting Started Guide][passage-guide] on our website. You
can also find more information on how to configure, optimize and embed Passage in your network there.

## Reporting Security Issues

To report a security issue for this project, please note our [Security Policy][security-policy].

## Code of Conduct

Participation in this project comes under the [Contributor Covenant Code of Conduct][code-of-conduct].

## How to contribute

Thanks for considering contributing to this project! In order to submit a Pull Request, please read
our [contributing][contributing-guide] guide. This project is in active development, and we're always happy to receive
new contributions!

## License

This project is developed and distributed under the MIT License. See [this explanation][mit-license-doc] for a rundown
on what that means.

[passage-website]: https://passage.scrayos.net

[passage-guide]: https://passage.scrayos.net/docs/getting-started

[passage-comparison]: https://passage.scrayos.net/docs/comparison

[chat-signing-explanation]:https://gist.github.com/kennytv/ed783dd244ca0321bbd882c347892874

[protocol-docs]: https://minecraft.wiki/w/Java_Edition_protocol/Packets

[minecraft-1-20-5]: http://minecraft.wiki/w/1.20.5

[rust-docs]: https://www.rust-lang.org/

[kubernetes-docs]: https://kubernetes.io/

[pvn-docs]: https://wiki.vg/Protocol_version_numbers

[transfer-packet]: https://minecraft.wiki/w/Java_Edition_protocol/Packets#Transfer_(configuration)

[bungeecord-docs]: https://github.com/SpigotMC/BungeeCord

[waterfall-docs]: https://github.com/PaperMC/Waterfall

[velocity-docs]: https://github.com/PaperMC/Velocity

[semver-docs]: https://semver.org/lang/de/

[github-releases]: https://github.com/scrayosnet/passage/releases

[github-ghcr]: https://github.com/scrayosnet/passage/pkgs/container/passage

[helm-chart-docs]: https://helm.sh/

[kustomize-docs]: https://kustomize.io/

[security-policy]: SECURITY.md

[code-of-conduct]: CODE_OF_CONDUCT.md

[contributing-guide]: CONTRIBUTING.md

[mit-license-doc]: https://choosealicense.com/licenses/mit/
