import { describe, expect, it } from "vitest";
import type { PluginItem, SkillItem } from "./api";
import { filterPlugins, filterSkills } from "./inventory-filter";

const plugins: PluginItem[] = [
  {
    id: "docs@personal",
    name: "OpenAI Docs",
    version: "1.0.0",
    description: "官方文档",
    enabled: true,
    marketplace: "personal",
    path: "C:\\docs",
    source: "command",
    error: null,
  },
  {
    id: "review@team",
    name: "Review",
    version: null,
    description: "团队审查",
    enabled: false,
    marketplace: "team",
    path: null,
    source: "filesystem",
    error: null,
  },
];

const skills: SkillItem[] = [
  {
    id: "release",
    name: "Release Notes",
    description: "发布检查",
    origin: "personal",
    pluginName: null,
    path: "C:\\release",
    source: "filesystem",
    error: null,
  },
  {
    id: "review/security",
    name: "Security Pass",
    description: "安全检查",
    origin: "plugin",
    pluginName: "Review",
    path: "C:\\review\\security",
    source: "filesystem",
    error: null,
  },
];

describe("filterPlugins", () => {
  it("combines text search and enabled state", () => {
    expect(filterPlugins(plugins, "docs", "enabled").map((item) => item.id)).toEqual(["docs@personal"]);
    expect(filterPlugins(plugins, "团队", "disabled").map((item) => item.id)).toEqual(["review@team"]);
  });
});

describe("filterSkills", () => {
  it("searches plugin names and filters origins", () => {
    expect(filterSkills(skills, "review", "plugin").map((item) => item.id)).toEqual(["review/security"]);
    expect(filterSkills(skills, "发布", "personal").map((item) => item.id)).toEqual(["release"]);
  });
});
