import { ArrowSquareOut, GithubLogo } from "@/components/icons"
import { Button } from "@/components/ui/button"
import { openExternalUrl } from "@/lib/os"
import { SettingRow, SettingsGroup } from "../primitives"

interface Contributor {
  name: string
  handle: string
  avatar: string
  role: string
  profileUrl: string
}

const CONTRIBUTORS: Contributor[] = [
  {
    name: "tcmmi",
    handle: "tcmmi",
    avatar: "https://avatars.githubusercontent.com/u/119017174?v=4",
    role: "Creator, media engine, models, and core architecture",
    profileUrl: "https://github.com/tcmmi",
  },
  {
    name: "0nsku",
    handle: "0nsku",
    avatar: "https://avatars.githubusercontent.com/u/143646543?v=4",
    role: "Core engineering, anime benchmarks, and analysis",
    profileUrl: "https://github.com/0nsku",
  },
  {
    name: "ihyledd",
    handle: "ihyledd",
    avatar: "https://avatars.githubusercontent.com/u/156428574?v=4",
    role: "macOS port, settings redesign, and project layout",
    profileUrl: "https://github.com/ihyledd",
  },
  {
    name: "Moongetsu",
    handle: "moongetsu",
    avatar: "https://avatars.githubusercontent.com/u/111883135?v=4",
    role: "UI design, custom context menus, and visual polish",
    profileUrl: "https://github.com/moongetsu",
  },
]

const REPO_URL = "https://github.com/tcmmi/AutoScene"

export function ContributorsSection() {
  return (
    <>
      <div className="relative mb-10 flex items-center justify-between gap-6 overflow-hidden rounded-3xl border border-white/[0.08] bg-[radial-gradient(120%_140%_at_0%_0%,rgb(224_180_92/0.16),transparent_55%),rgb(255_255_255/0.03)] p-7">
        <div className="min-w-0">
          <h2 className="display text-3xl text-fg-1">Community Contributors</h2>
          <p className="mt-1 text-md text-fg-2">
            AutoScene is made possible by open-source contributors and community feedback.
          </p>
        </div>
        <Button size="sm" onClick={() => void openExternalUrl(REPO_URL)}>
          <GithubLogo /> Contribute on GitHub
        </Button>
      </div>

      <SettingsGroup
        title="Contributors"
        description="People who have shaped AutoScene with code, design, and ideas."
      >
        {CONTRIBUTORS.map((c) => (
          <SettingRow
            key={c.handle}
            label={
              <div className="flex items-center gap-3">
                <img
                  src={c.avatar}
                  alt={c.name}
                  className="size-8 shrink-0 rounded-full border border-white/[0.1] bg-white/[0.05] object-cover"
                  loading="lazy"
                />
                <div>
                  <span className="font-medium text-fg-1">{c.name}</span>
                  <span className="ml-2 font-mono text-xs text-fg-3">@{c.handle}</span>
                </div>
              </div>
            }
            description={<span className="block mt-1 text-fg-2">{c.role}</span>}
            className="min-h-16 py-3"
          >
            <Button
              size="sm"
              variant="ghost"
              onClick={() => void openExternalUrl(c.profileUrl)}
              aria-label={`Open ${c.name}'s GitHub profile`}
            >
              <ArrowSquareOut /> GitHub
            </Button>
          </SettingRow>
        ))}
      </SettingsGroup>
    </>
  )
}

