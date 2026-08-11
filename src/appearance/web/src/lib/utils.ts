// The `cn` helper every shadcn component imports: merge conditional class names,
// with later Tailwind utilities winning over earlier conflicting ones.
import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
