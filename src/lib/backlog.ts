// Display metadata for backlog categories — labels, colors, and the
// canonical priority order PO sees on the backend side. Used by both the
// BacklogPanel and the compose forms (PresentingOverlay, Dashboard).

import type { BacklogCategory } from "../types";

export interface CategoryMeta {
  /** Short Russian label shown in selectors and badges. */
  label: string;
  /** Single-emoji marker used in badges and dropdowns. */
  emoji: string;
  /** Tailwind classes for badge background/text/border. */
  color: string;
  /** Short hint shown next to the option when the user picks a category. */
  hint: string;
}

export const CATEGORY_META: Record<BacklogCategory, CategoryMeta> = {
  critical: {
    label: "Крит",
    emoji: "🚨",
    color: "bg-destructive/10 text-destructive border-destructive/40",
    hint: "блокер: проект не работает / билд упал / демо мертво",
  },
  bug: {
    label: "Баг",
    emoji: "🐛",
    color: "bg-warning/10 text-warning border-warning/40",
    hint: "сломанный сценарий, ошибка",
  },
  tech_debt: {
    label: "Техдолг",
    emoji: "🔧",
    color: "bg-info/10 text-info border-info/40",
    hint: "риск, недоделка, рефакторинг",
  },
  feature: {
    label: "Фича",
    emoji: "✨",
    color: "bg-primary/10 text-primary border-primary/40",
    hint: "новый функционал",
  },
  wish: {
    label: "Идея",
    emoji: "💭",
    color: "bg-muted text-muted-foreground border-border",
    hint: "пожелание / nice-to-have",
  },
};

/** Category order used when rendering selectors — matches PO priority. */
export const CATEGORY_ORDER: readonly BacklogCategory[] = [
  "critical",
  "bug",
  "tech_debt",
  "feature",
  "wish",
];
