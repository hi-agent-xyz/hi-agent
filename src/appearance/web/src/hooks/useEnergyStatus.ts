import { useEffect, useState } from "react";

interface EnergyResponse {
  out_of_energy?: boolean;
}

/**
 * Keep availability in the session layer so the status control is the only
 * visible treatment of the account's energy state.
 */
export function useEnergyStatus(): boolean {
  const [outOfEnergy, setOutOfEnergy] = useState(false);

  useEffect(() => {
    let alive = true;
    let knownOut = false;
    let timer: number | undefined;

    const schedule = () => {
      if (alive) timer = window.setTimeout(run, 5000);
    };

    const run = async () => {
      timer = undefined;
      try {
        // Refresh the real broker balance while paused. The first positive
        // result broadcasts Resume in the host and wakes held sessions.
        const url = knownOut
          ? "/api/account/energy?refresh=true"
          : "/api/account/energy";
        const response = await fetch(url);
        if (response.ok) {
          const data = (await response.json()) as EnergyResponse;
          if (alive) {
            knownOut = data.out_of_energy === true;
            setOutOfEnergy(knownOut);
          }
        }
      } catch {
        // A transient account check failure does not change the last known state.
      }
      schedule();
    };

    void run();

    return () => {
      alive = false;
      if (timer !== undefined) window.clearTimeout(timer);
    };
  }, []);

  return outOfEnergy;
}
