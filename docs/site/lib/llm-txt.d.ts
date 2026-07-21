export const basePath: string;
export const site: string;
export function collectPages(base?: string): { section: string; title: string; slug: string; txt: string }[];
export function llmsTxt(base?: string): string;
