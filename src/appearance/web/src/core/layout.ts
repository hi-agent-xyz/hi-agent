// The compositor: one pure pass that arranges the host's own live surfaces —
// the caption words and the camera self-view — around whatever the agent has on
// screen. Deterministic (no solver, no async coordinator).
//
// It no longer *places* agent views. Views are full-bleed, one at a time, so
// there is nothing left to decide about where one goes; what remains is the
// question the host surfaces actually ask — does the camera fill the stage or
// tuck into its pip, do the words lead or dock, and does the top-most view
// render them itself. That is this module's whole job.
//
// Captions and camera are placed here but NEVER built or owned here: they stay
// host-rendered, pinned surfaces (the camera <video> keeps its srcObject; the
// caption DOM keeps streaming). This module only decides WHERE each box goes —
// the "placement, not lifecycle" line the whole design rests on.

/** Stable ids for the two host-rendered participants, so a placement map can key
 * them alongside the agent views without colliding with any view id. */
export const CAPTIONS_ID = "__captions__";
export const CAMERA_ID = "__camera__";

export type ParticipantKind = "view" | "captions" | "camera";

/** Where a host surface sits. Views aren't placed — they fill the frame — so this
 * is only ever the handful of spots the captions and the camera use. */
export type Region = "center" | "bottom" | "bottom_left" | "fill";

/** One thing competing for the stage. A `view` carries `ownsCaptions` (all a view
 * declares now); the host chrome carries nothing — the compositor supplies its
 * defaults. */
export interface Participant {
  id: string;
  kind: ParticipantKind;
  ownsCaptions?: boolean;
}

/** Where the compositor decided one participant sits. The three flags are
 * kind-specific (default false): `hidden` suppresses captions a view renders
 * itself; `pip` shrinks the camera to its corner; `docked` marks captions as
 * pills over content (vs. centered as the lead). */
export interface Placement {
  id: string;
  kind: ParticipantKind;
  region: Region;
  hidden: boolean;
  pip: boolean;
  docked: boolean;
}

/** The whole stage resolved: how far to fade the presence while content leads,
 * plus a placement per host surface. */
export interface Layout {
  demote: number;
  placements: Map<string, Placement>;
}

/**
 * Arrange the host surfaces around whatever is on screen. The presence demotes
 * once any view leads, the camera fills the stage alone but shrinks to a pip
 * behind a view, and the captions dock as pills whenever something fills the
 * stage — centered as the lead otherwise.
 */
export function floorLayout(participants: Participant[]): Layout {
  const views = participants.filter((p) => p.kind === "view");
  const captions = participants.find((p) => p.kind === "captions");
  const camera = participants.find((p) => p.kind === "camera");

  const overlaid = views.length > 0;
  const demote = overlaid ? 0.72 : 0;
  const placements = new Map<string, Placement>();

  // The camera fills the stage as a backdrop when nothing leads; behind a view it
  // tucks into the lower-left corner pip.
  const cameraFill = !!camera && !overlaid;
  if (camera) {
    placements.set(camera.id, {
      id: camera.id,
      kind: "camera",
      region: cameraFill ? "fill" : "bottom_left",
      hidden: false,
      pip: overlaid,
      docked: false,
    });
  }

  // The words dock as a single bottom-center strip whenever something fills the
  // stage behind them — a view, or the camera-as-backdrop — sat between the
  // camera pip (left) and the controls (right) so they never cover the content.
  // The top-most view may still render the words itself, in which case the host
  // stands down. Alone (nothing on the stage), the words are the lead and centre.
  if (captions) {
    const docked = overlaid || cameraFill;
    const top = views[views.length - 1];
    placements.set(captions.id, {
      id: captions.id,
      kind: "captions",
      region: docked ? "bottom" : "center",
      hidden: overlaid && !!top?.ownsCaptions,
      pip: false,
      docked,
    });
  }

  return { demote, placements };
}
