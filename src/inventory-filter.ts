import type { PluginItem, SkillItem } from "./api";

export type PluginFilter = "all" | "enabled" | "disabled";
export type SkillFilter = "all" | "personal" | "plugin" | "system";

export interface PluginSkillGroup {
  pluginName: string;
  items: SkillItem[];
}

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

export function groupSkillsByPlugin(items: SkillItem[]): {
  standalone: SkillItem[];
  pluginGroups: PluginSkillGroup[];
} {
  const standalone: SkillItem[] = [];
  const groups = new Map<string, PluginSkillGroup>();

  for (const item of items) {
    if (item.origin !== "plugin") {
      standalone.push(item);
      continue;
    }

    const pluginName = item.pluginName?.trim() || "未识别插件";
    const key = pluginName.toLocaleLowerCase("zh-CN");
    const group = groups.get(key);
    if (group) group.items.push(item);
    else groups.set(key, { pluginName, items: [item] });
  }

  return {
    standalone,
    pluginGroups: [...groups.values()].sort((left, right) =>
      left.pluginName.localeCompare(right.pluginName, "zh-CN"),
    ),
  };
}

function normalize(value: string): string {
  return value.trim().toLocaleLowerCase("zh-CN");
}
