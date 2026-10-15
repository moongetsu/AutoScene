import { describe, expect, it } from "vitest"
import type { MediaInfo } from "@/bindings"
import { audioLabel, audioOption, releaseTitle } from "./media"

describe("releaseTitle", () => {
  it.each([
    ["Breaking Bad S01E01 1080p BluRay REMUX AVC [Org TV-DL Hindi DD 2.0 ~ 224Kbps + English DTS-HD MA 5.1] ESubs ~ (ARRØW) FraMeSToR.mkv", "Breaking Bad"],
    ["The.Office.US.S02E03.Office.Olympics.720p.WEB-DL.mkv", "Office Olympics"],
    ["[SubsPlease] Sousou no Frieren - 03 (1080p) [ABCD1234].mkv", "Sousou no Frieren"],
    ["Blade.Runner.2049.2017.2160p.UHD.BluRay.x265.mkv", "Blade Runner 2049"],
    ["My holiday clip.mp4", "My holiday clip"],
    ["S01E05.mkv", "S01E05"],
  ])("%s", (file, want) => {
    expect(releaseTitle(file)).toBe(want)
  })
})

describe("audio track labels", () => {
  const track = (o: Partial<MediaInfo["audio"][number]>): MediaInfo["audio"][number] => ({
    index: 1,
    ordinal: 0,
    codec: "ac3",
    profile: null,
    channels: 2,
    channelLayout: null,
    sampleRate: 48000,
    language: null,
    title: null,
    default: true,
    commentary: false,
    ...o,
  })

  it("leads with the language and names the format", () => {
    expect(audioLabel(track({ language: "hin" }))).toBe("Hindi · Dolby Digital · Stereo")
    expect(audioLabel(track({ language: "eng", codec: "dts", profile: "DTS-HD MA", channels: 6 }))).toBe(
      "English · DTS-HD MA · 5.1",
    )
    expect(audioLabel(track({ ordinal: 2, codec: "aac", commentary: true }))).toBe("Track 3 · AAC · Stereo · Commentary")
  })

  it("shows the file's own title only when it adds something", () => {
    expect(audioOption(track({ language: "eng", codec: "dts", profile: "DTS-HD MA", channels: 6, title: "DTS-HD MA 5.1" })).description).toBeUndefined()
    expect(audioOption(track({ language: "hin", title: "Org TV-DL Hindi DD 2.0Ch 224Kbps" })).description).toBe(
      "Org TV-DL Hindi DD 2.0Ch 224Kbps",
    )
  })
})
