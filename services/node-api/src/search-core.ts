import fs from 'node:fs';

export type Category =
  | 'template'
  | 'character'
  | 'background'
  | 'effect'
  | 'prop'
  | 'speechBubble'
  | 'textTemplate';

export interface SeedItem {
  id: number;
  category: Category;
  role: string;
  language: string;
  title: string;
  tags: string[];
  popularity: number;
  updatedAt: string;
  ownerUserId: number;
  purchasedBy: number[];
}

export interface CategoryJob {
  category: Category;
  tokens: string[];
  role: string;
  language: string;
  perCategoryLimit: number;
}

export interface FanOutHit {
  id: number;
  category: Category;
  title: string;
  popularity: number;
  updatedAt: string;
  ownerUserId: number;
  purchasedBy: number[];
  score: number;
}

export const categories: Category[] = [
  'template',
  'character',
  'background',
  'effect',
  'prop',
  'speechBubble',
  'textTemplate'
];

export function loadSeed(datasetPath: string, datasetMultiplier: number): SeedItem[] {
  const baseSeedData = JSON.parse(fs.readFileSync(datasetPath, 'utf-8')) as SeedItem[];
  return Array.from({ length: datasetMultiplier }).flatMap((_, idx) =>
    baseSeedData.map((item) => ({
      ...item,
      id: item.id + idx * 100000,
      popularity: item.popularity + (idx % 10),
      title: `${item.title} #${idx}`
    }))
  );
}

export function tokenizeTagText(tagText: string): string[] {
  return tagText
    .toLowerCase()
    .split(/\s+/)
    .map((x) => x.trim())
    .filter(Boolean);
}

export function scoreItem(item: SeedItem, tokens: string[]): number {
  let tagMatchCount = 0;
  for (const token of tokens) {
    if (item.tags.some((tag) => tag.includes(token))) {
      tagMatchCount += 1;
    }
  }

  const titleBonus = tokens.some((token) => item.title.toLowerCase().includes(token)) ? 5 : 0;
  const freshnessScore = 20 / (1 + (item.id % 30));
  return tagMatchCount * 10 + item.popularity * 0.1 + titleBonus + freshnessScore;
}

export function fanOutCategory(seed: SeedItem[], job: CategoryJob): FanOutHit[] {
  const scored: FanOutHit[] = [];
  for (const item of seed) {
    if (item.category !== job.category) continue;
    if (job.role !== 'all' && item.role !== job.role) continue;
    if (item.language !== job.language) continue;
    const matched = job.tokens.every(
      (token) =>
        item.tags.some((tag) => tag.includes(token)) || item.title.toLowerCase().includes(token)
    );
    if (!matched) continue;
    scored.push({
      id: item.id,
      category: item.category,
      title: item.title,
      popularity: item.popularity,
      updatedAt: item.updatedAt,
      ownerUserId: item.ownerUserId,
      purchasedBy: item.purchasedBy,
      score: scoreItem(item, job.tokens)
    });
  }

  scored.sort((a, b) => b.score - a.score);
  return scored.slice(0, job.perCategoryLimit * 2);
}
