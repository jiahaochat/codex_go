import type { PluginItem, SkillItem } from "./api";

export type PluginFilter = "all" | "enabled" | "disabled";
export type SkillFilter = "all" | "personal" | "plugin" | "system";

export function filterPlugins(items: PluginItem[], query: string, filter: PluginFilter): PluginItem[] {
  const needle = normalize(query);
  return items.filter((item) => {
    const haystack = normalize(`${item.name} ${item.description ?? ""} ${item.marketplace ?? ""}`);
    const matchesQuery = !needle || haystack.includes(needle);
    const matchesFilter = filter === "all" || (filter === "enabled" ? item.enabled : !item.enabled);
    return matchesQuery && matchesFilter;
  });
}

export function filterSkills(items: SkillItem[], query: string, filter: SkillFilter): SkillItem[] {
  const needle = normalize(query);
  return items.filter((item) => {
    const haystack = normalize(`${item.name} ${item.description ?? ""} ${item.pluginName ?? ""}`);
    const matchesQuery = !needle || haystack.includes(needle);
    return matchesQuery && (filter === "all" || item.origin === filter);
  });
}

function normalize(value: string): string {
  return value.trim().toLocaleLowerCase("zh-CN");
}
