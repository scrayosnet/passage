# Passage Documentation

The source of [passage.scrayos.net](https://passage.scrayos.net), built with [Astro](https://astro.build) and
[Starlight](https://starlight.astro.build).

## Structure

All pages live in `src/content/docs/` and are routed by their file path — `src/content/docs/setup/installation.md`
becomes `/setup/installation/`. The sidebar is generated automatically per directory; use the `sidebar.order`
frontmatter field to control the position of a page within its group.

| Directory   | Content                                                             |
|-------------|---------------------------------------------------------------------|
| `overview/` | Introduction, architecture, security and proxy comparison            |
| `setup/`    | Installation, configuration basics and Kubernetes deployment         |
| `adapters/` | Reference for the status, authentication and discovery adapters      |
| `advanced/` | Cookies, localization, observability, tracing, scaling, gRPC adapters |
| `reference/`| Full configuration and gRPC protocol reference                       |

Images belong in `src/assets/` (referenced relatively from Markdown), static files such as the favicon and
`robots.txt` in `public/`. Site-wide settings — title, sidebar groups, social links, plugins — are configured in
`astro.config.mjs`.

## Commands

Run from this directory (`.docs/`):

| Command        | Action                                              |
|----------------|-----------------------------------------------------|
| `pnpm install` | Install dependencies                                |
| `pnpm dev`     | Start the dev server at `localhost:4321`            |
| `pnpm build`   | Build the production site to `./dist/`              |
| `pnpm preview` | Preview the production build locally                |

## Contributing

Documentation changes follow the same process as code changes — see [CONTRIBUTING.md](../CONTRIBUTING.md). When you
document a configuration field, verify it against `src/config.rs` and the generated `config/schema.json`, so the
reference and the implementation stay in sync.
