/**
 * Where a horizontally scrolling strip has to be scrolled to have one of its cards on
 * screen — or `null` when it is on screen already and the answer is to leave it alone.
 *
 * Separated from the band it serves because it is arithmetic, and arithmetic done
 * inside a layout effect is arithmetic nobody can test: `offsetLeft` and `clientWidth`
 * are all zero under jsdom, so the only way to check the sums is to hand them in.
 *
 * A card that is fully visible is left where it is, so opening the band on the common
 * case — the cursor at the head of the row — does not scroll at all. A card that is not
 * gets centred rather than nudged to the edge, because the reason to look at it is
 * usually to compare it with what is around it.
 */
export function scrollToShow(
  strip: { scrollLeft: number; clientWidth: number },
  card: { offsetLeft: number; offsetWidth: number },
): number | null {
  const right = card.offsetLeft + card.offsetWidth;
  const onScreen =
    card.offsetLeft >= strip.scrollLeft && right <= strip.scrollLeft + strip.clientWidth;
  if (onScreen) return null;
  return Math.max(0, card.offsetLeft - (strip.clientWidth - card.offsetWidth) / 2);
}
