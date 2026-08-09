import { useEffect, useState } from "react";
import type {
  AttentionItem,
  Crop,
  HarvestGroup,
  HarvestInput,
  RecountResult,
  TodayView,
} from "@/farm/types";
import {
  advanceTrays,
  checkAttention,
  dismissAttention,
  farmLocation,
  harvestGroups,
  listCrops,
  pollStripe,
  resolveAttention,
  sowTray,
  takeNewPaidOrders,
  todayView,
  undoLast,
} from "@/farm/api";
import { formatClock, parseLocalDate, weekdayName } from "@/farm/dates";
import { Card } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { SowSheet } from "@/components/SowSheet";
import { WeightPad } from "@/components/WeightPad";
import { FarmBackupSheet } from "@/components/FarmBackupSheet";
import {
  RecountSheet,
  recountResultMessage,
} from "@/components/RecountSheet";
import { SellOnlineSheet } from "@/components/SellOnlineSheet";
import { MoneyJustLeftSheet } from "@/components/MoneyJustLeftSheet";
import { SeedRatesSheet } from "@/components/SeedRatesSheet";
import { MilesSheet } from "@/components/MilesSheet";
import { EquipmentSheet } from "@/components/EquipmentSheet";

function trayCountLabel(quantity: number): string {
  return `${quantity} ${quantity === 1 ? "tray" : "trays"}`;
}

const actionCardClass =
  "flex min-h-24 cursor-pointer items-center justify-center p-8 text-center text-xl font-medium transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none";

const confirmCardClass =
  "flex min-h-24 items-center justify-between gap-4 p-8 text-xl font-medium";

type LastAction =
  | { kind: "moved"; trayCount: number }
  | {
      kind: "harvested";
      trayCount: number;
      varietyCount: number;
      cropName: string | null;
      yieldOz: number;
    }
  | { kind: "discarded"; trayCount: number; cropName: string }
  | { kind: "restored"; label: string }
  | { kind: "recounted"; message: string; canUndo: boolean }
  | { kind: "attention_dismissed" };

export function Today() {
  const [crops, setCrops] = useState<Crop[]>([]);
  const [view, setView] = useState<TodayView | null>(null);
  const [attention, setAttention] = useState<AttentionItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [sheetOpen, setSheetOpen] = useState(false);
  const [lastAction, setLastAction] = useState<LastAction | null>(null);
  const [harvestOpen, setHarvestOpen] = useState(false);
  const [harvestGroupsForPad, setHarvestGroupsForPad] = useState<
    HarvestGroup[] | null
  >(null);
  const [backupOpen, setBackupOpen] = useState(false);
  const [lastBackupAt, setLastBackupAt] = useState<Date | null>(null);
  const [recountOpen, setRecountOpen] = useState(false);
  const [sellOpen, setSellOpen] = useState(false);
  const [seedRatesOpen, setSeedRatesOpen] = useState(false);
  const [moneyLeftOpen, setMoneyLeftOpen] = useState(false);
  const [deliveryCostOpen, setDeliveryCostOpen] = useState(false);
  const [milesOpen, setMilesOpen] = useState(false);
  const [equipmentOpen, setEquipmentOpen] = useState(false);
  const [newPaidCount, setNewPaidCount] = useState(0);

  async function refreshAttention() {
    const items = await checkAttention();
    setAttention(items);
  }

  async function refresh() {
    const v = await todayView();
    setView(v);
    await refreshAttention();
  }

  /** Poll never blocks the first paint; failures stay off the critical path. */
  async function runPollCycle(options?: { stampOpen?: boolean }) {
    try {
      await pollStripe();
      if (options?.stampOpen) {
        const n = await takeNewPaidOrders();
        setNewPaidCount(n.count);
      }
      await refresh();
      await refreshBackupLine();
    } catch (err) {
      console.error(err);
      try {
        await refresh();
      } catch (e) {
        console.error(e);
      }
    }
  }

  async function refreshBackupLine() {
    try {
      const loc = await farmLocation();
      setLastBackupAt(
        loc.lastSnapshotAt ? new Date(loc.lastSnapshotAt) : null,
      );
    } catch (err) {
      console.error(err);
    }
  }

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [c, v, items] = await Promise.all([
          listCrops(),
          todayView(),
          checkAttention(),
        ]);
        if (!cancelled) {
          setCrops(c);
          setView(v);
          setAttention(items);
        }
        await refreshBackupLine();
      } catch (err) {
        console.error(err);
      } finally {
        if (!cancelled) setLoading(false);
      }
      // On open: poll, then count new paid orders. Never blocks the window.
      if (!cancelled) {
        void runPollCycle({ stampOpen: true });
      }
    })();

    const interval = window.setInterval(() => {
      void runPollCycle();
    }, 60_000);

    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, []);

  async function handleRestored(label: string) {
    setLastAction({ kind: "restored", label });
    try {
      await refresh();
      await refreshBackupLine();
    } catch (err) {
      console.error(err);
    }
  }

  async function handleAttentionDismiss(id: string) {
    try {
      await dismissAttention(id);
      setLastAction({ kind: "attention_dismissed" });
      await refresh();
    } catch (err) {
      console.error(err);
    }
  }

  async function handleAttentionAction(item: AttentionItem, action: string) {
    if (action === "dismiss") {
      await handleAttentionDismiss(item.id);
      return;
    }
    try {
      const result = await resolveAttention(item.id, action);
      if (action === "try_now" && item.kind === "poll.failed") {
        await refresh();
        return;
      }
      if (action === "open_in_stripe" && result.openUrl) {
        window.open(result.openUrl, "_blank", "noopener,noreferrer");
        await refresh();
        return;
      }
      if (action === "move_now" && result.trayIds.length > 0) {
        const trayCount =
          view?.moveToLight &&
          result.trayIds.every((id) => view.moveToLight!.trayIds.includes(id))
            ? view.moveToLight.trayCount
            : result.trayIds.length;
        await advanceTrays(result.trayIds);
        setLastAction({ kind: "moved", trayCount });
        await refresh();
        return;
      }
      if (action === "harvest_now") {
        const v = await todayView();
        setView(v);
        await refreshAttention();
        const idSet = new Set(result.trayIds);
        const groups = v.harvests.filter((g) =>
          g.trayIds.some((id) => idSet.has(id)),
        );
        setHarvestGroupsForPad(groups.length > 0 ? groups : v.harvests);
        setHarvestOpen(true);
        return;
      }
      await refresh();
    } catch (err) {
      console.error(err);
      try {
        await refresh();
      } catch (e) {
        console.error(e);
      }
    }
  }

  async function handleSow(
    crop: Crop,
    quantity: number,
    seedOz: number | null,
  ) {
    try {
      await sowTray(crop.id, quantity, seedOz);
      await refresh();
      setSheetOpen(false);
    } catch (err) {
      console.error(err);
      try {
        await refresh();
      } catch (e) {
        console.error(e);
      }
    }
  }

  async function handleMoveToLight(trayIds: string[], trayCount: number) {
    try {
      await advanceTrays(trayIds);
      setLastAction({ kind: "moved", trayCount });
      await refresh();
    } catch (err) {
      console.error(err);
      try {
        await refresh();
      } catch (e) {
        console.error(e);
      }
    }
  }

  async function handleHarvestDone(
    groups: HarvestInput[],
    meta: { trayCount: number; varietyCount: number; cropName: string | null },
  ) {
    try {
      await harvestGroups(groups);
      const yieldOz = groups.reduce((s, g) => s + g.actualYieldOz, 0);
      setLastAction({
        kind: "harvested",
        trayCount: meta.trayCount,
        varietyCount: meta.varietyCount,
        cropName: meta.cropName,
        yieldOz,
      });
      setHarvestOpen(false);
      setHarvestGroupsForPad(null);
      await refresh();
    } catch (err) {
      console.error(err);
      try {
        await refresh();
      } catch (e) {
        console.error(e);
      }
    }
  }

  function handleDiscarded(info: { trayCount: number; cropName: string }) {
    setLastAction({
      kind: "discarded",
      trayCount: info.trayCount,
      cropName: info.cropName,
    });
  }

  async function handleHarvestOpenChange(open: boolean) {
    setHarvestOpen(open);
    if (!open) {
      setHarvestGroupsForPad(null);
      try {
        await refresh();
      } catch (err) {
        console.error(err);
      }
    }
  }

  async function handleUndo() {
    try {
      await undoLast();
      setLastAction(null);
      await refresh();
    } catch (err) {
      console.error(err);
      try {
        await refresh();
      } catch (e) {
        console.error(e);
      }
    }
  }

  async function handleRecountDone(result: RecountResult, cropCount: number) {
    const { matched, text } = recountResultMessage(result, cropCount);
    setLastAction({ kind: "recounted", message: text, canUndo: !matched });
    try {
      await refresh();
    } catch (err) {
      console.error(err);
    }
  }

  function attentionActionLabel(action: string): string {
    switch (action) {
      case "try_now":
        return "Try now";
      case "harvest_now":
        return "Harvest";
      case "move_now":
        return "Move to light";
      case "dismiss":
        return "Dismiss";
      default:
        return action;
    }
  }

  const hasActionRows =
    !!view?.moveToLight ||
    !!view?.harvestSummary ||
    (lastAction !== null &&
      lastAction.kind !== "restored" &&
      !(lastAction.kind === "recounted" && !lastAction.canUndo));

  const backupLine = lastBackupAt
    ? `Farm saved automatically · last backup ${formatClock(lastBackupAt)}`
    : "Farm saved automatically · last backup —";

  function harvestRowLabel(): string {
    const hs = view!.harvestSummary!;
    if (hs.varietyCount === 1 && hs.singleCropName) {
      return `Harvest today — ${trayCountLabel(hs.trayCount)} of ${hs.singleCropName}, est. ${hs.estimatedYieldOz.toFixed(1)} oz`;
    }
    return `Harvest today — ${trayCountLabel(hs.trayCount)}, ${hs.varietyCount} varieties, est. ${hs.estimatedYieldOz.toFixed(1)} oz`;
  }

  function harvestConfirmLabel(action: Extract<LastAction, { kind: "harvested" }>): string {
    if (action.varietyCount === 1 && action.cropName) {
      return `Harvested ${trayCountLabel(action.trayCount)} of ${action.cropName} — ${action.yieldOz.toFixed(1)} oz.`;
    }
    return `Harvested ${trayCountLabel(action.trayCount)} across ${action.varietyCount} varieties — ${action.yieldOz.toFixed(1)} oz.`;
  }

  return (
    <main className="mx-auto flex min-h-screen w-full max-w-md flex-col gap-8 px-6 py-12">
      <h1 className="text-3xl font-semibold tracking-tight">Today</h1>

      {!loading && attention.length > 0 && (
        <div className="flex flex-col gap-6">
          {attention.map((item) => (
            <Card key={item.id} className="flex flex-col gap-4 p-6">
              <p className="text-base font-medium leading-snug">{item.message}</p>
              <div className="flex flex-wrap gap-2">
                {item.actions
                  .filter((a) => a !== "dismiss")
                  .map((action) => (
                    <Button
                      key={action}
                      type="button"
                      className="h-11 px-4 text-base"
                      onClick={() => void handleAttentionAction(item, action)}
                    >
                      {attentionActionLabel(action)}
                    </Button>
                  ))}
                <Button
                  type="button"
                  variant="ghost"
                  className="h-11 px-4 text-base"
                  onClick={() => void handleAttentionDismiss(item.id)}
                >
                  Dismiss
                </Button>
              </div>
            </Card>
          ))}
        </div>
      )}

      {!loading && view && (
        <>
          {view.activeTrayCount === 0 &&
          (lastAction === null ||
            lastAction.kind === "restored" ||
            lastAction.kind === "attention_dismissed") ? (
            // State A — Script A empty state, unchanged
            <div className="flex flex-col gap-6">
              {lastAction?.kind === "restored" && (
                <Card className={confirmCardClass}>
                  <span>Farm restored from {lastAction.label}.</span>
                </Card>
              )}
              {lastAction?.kind === "attention_dismissed" && (
                <Card className={confirmCardClass}>
                  <span>Dismissed.</span>
                  <Button
                    type="button"
                    variant="ghost"
                    onClick={handleUndo}
                    className="h-14 px-3 text-base"
                  >
                    Undo
                  </Button>
                </Card>
              )}
              <Card
                role="button"
                tabIndex={0}
                onClick={() => setSheetOpen(true)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    setSheetOpen(true);
                  }
                }}
                className="flex min-h-24 cursor-pointer items-center justify-center p-8 text-xl font-medium transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
              >
                Sow your first tray
              </Card>
              <Card
                role="button"
                tabIndex={0}
                onClick={() => setDeliveryCostOpen(true)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    setDeliveryCostOpen(true);
                  }
                }}
                className="flex min-h-24 cursor-pointer items-center justify-center p-8 text-center text-xl font-medium transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
              >
                Money out for a delivery run
              </Card>
              <Card
                role="button"
                tabIndex={0}
                onClick={() => setMilesOpen(true)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    setMilesOpen(true);
                  }
                }}
                className="flex min-h-24 cursor-pointer items-center justify-center p-8 text-center text-xl font-medium transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
              >
                Log miles
              </Card>
              <button
                type="button"
                onClick={() => setMoneyLeftOpen(true)}
                className="flex min-h-14 items-center justify-center rounded-xl border px-4 text-base font-medium transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
              >
                Money just left
              </button>
              <button
                type="button"
                onClick={() => setSeedRatesOpen(true)}
                className="flex min-h-11 items-center text-left text-sm text-muted-foreground underline-offset-4 hover:underline focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
              >
                Seed rates
              </button>
              <button
                type="button"
                onClick={() => setEquipmentOpen(true)}
                className="flex min-h-11 items-center text-left text-sm text-muted-foreground underline-offset-4 hover:underline focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
              >
                Equipment
              </button>
              <button
                type="button"
                onClick={() => setBackupOpen(true)}
                className="text-left text-sm text-muted-foreground underline-offset-4 hover:underline focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
              >
                {backupLine}
              </button>
            </div>
          ) : (
            <div className="flex flex-col gap-6">
              {/* 1. Move to light (or its confirmation) */}
              {lastAction?.kind === "restored" ? (
                <Card className={confirmCardClass}>
                  <span>Farm restored from {lastAction.label}.</span>
                </Card>
              ) : null}
              {lastAction?.kind === "recounted" ? (
                <Card className={confirmCardClass}>
                  <span>{lastAction.message}</span>
                  {lastAction.canUndo && (
                    <Button
                      type="button"
                      variant="ghost"
                      onClick={handleUndo}
                      className="h-14 px-3 text-base"
                    >
                      Undo
                    </Button>
                  )}
                </Card>
              ) : null}
              {lastAction?.kind === "attention_dismissed" ? (
                <Card className={confirmCardClass}>
                  <span>Dismissed.</span>
                  <Button
                    type="button"
                    variant="ghost"
                    onClick={handleUndo}
                    className="h-14 px-3 text-base"
                  >
                    Undo
                  </Button>
                </Card>
              ) : null}
              {lastAction?.kind === "moved" ? (
                <Card className={confirmCardClass}>
                  <span>
                    Moved {trayCountLabel(lastAction.trayCount)} to light.
                  </span>
                  <Button
                    type="button"
                    variant="ghost"
                    onClick={handleUndo}
                    className="h-14 px-3 text-base"
                  >
                    Undo
                  </Button>
                </Card>
              ) : (
                view.moveToLight && (
                  <Card
                    role="button"
                    tabIndex={0}
                    onClick={() =>
                      handleMoveToLight(
                        view.moveToLight!.trayIds,
                        view.moveToLight!.trayCount,
                      )
                    }
                    onKeyDown={(e) => {
                      if (e.key === "Enter" || e.key === " ") {
                        e.preventDefault();
                        handleMoveToLight(
                          view.moveToLight!.trayIds,
                          view.moveToLight!.trayCount,
                        );
                      }
                    }}
                    className={actionCardClass}
                  >
                    Move to light — {trayCountLabel(view.moveToLight.trayCount)}
                  </Card>
                )
              )}

              {/* 2. One Harvest today row (or confirmation / discard confirmation) */}
              {lastAction?.kind === "harvested" ? (
                <Card className={confirmCardClass}>
                  <span>{harvestConfirmLabel(lastAction)}</span>
                  <Button
                    type="button"
                    variant="ghost"
                    onClick={handleUndo}
                    className="h-14 px-3 text-base"
                  >
                    Undo
                  </Button>
                </Card>
              ) : lastAction?.kind === "discarded" ? (
                <Card className={confirmCardClass}>
                  <span>
                    Discarded {trayCountLabel(lastAction.trayCount)} of{" "}
                    {lastAction.cropName}.
                  </span>
                  <Button
                    type="button"
                    variant="ghost"
                    onClick={handleUndo}
                    className="h-14 px-3 text-base"
                  >
                    Undo
                  </Button>
                </Card>
              ) : (
                view.harvestSummary && (
                  <Card
                    role="button"
                    tabIndex={0}
                    onClick={() => setHarvestOpen(true)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" || e.key === " ") {
                        e.preventDefault();
                        setHarvestOpen(true);
                      }
                    }}
                    className={actionCardClass}
                  >
                    {harvestRowLabel()}
                  </Card>
                )
              )}

              {/* Idle states B / C — only when nothing is due */}
              {!hasActionRows &&
                view.sownToday &&
                view.nextEvent?.kind === "light" && (
                  // State B — Script A end state, byte-identical line
                  <Card className="flex flex-col gap-2 p-6">
                    <p className="text-xl font-medium">
                      {trayCountLabel(view.nextEvent.trayCount)} of{" "}
                      {view.nextEvent.cropName}
                    </p>
                    <p className="text-base text-muted-foreground">
                      {trayCountLabel(view.nextEvent.trayCount)} of{" "}
                      {view.nextEvent.cropName} · sown today · nothing to do
                      until{" "}
                      {weekdayName(parseLocalDate(view.nextEvent.date))} — cover
                      check
                    </p>
                  </Card>
                )}

              {!hasActionRows &&
                !(view.sownToday && view.nextEvent?.kind === "light") && (
                  // State C — plain-English next-event line
                  <p className="text-xl font-medium">
                    {view.nextEvent
                      ? view.nextEvent.kind === "light"
                        ? `Nothing to do today. Next: move ${trayCountLabel(view.nextEvent.trayCount)} to light on ${weekdayName(parseLocalDate(view.nextEvent.date))}.`
                        : `Nothing to do today. Next: harvest ${trayCountLabel(view.nextEvent.trayCount)} of ${view.nextEvent.cropName} on ${weekdayName(parseLocalDate(view.nextEvent.date))}.`
                      : "Nothing to do today. Nothing growing right now."}
                  </p>
                )}

              {/* 4. Sow more trays — permanent entry point when trays exist */}
              <Card
                role="button"
                tabIndex={0}
                onClick={() => setSheetOpen(true)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    setSheetOpen(true);
                  }
                }}
                className={actionCardClass}
              >
                Sow more trays
              </Card>

              <Card
                role="button"
                tabIndex={0}
                onClick={() => setDeliveryCostOpen(true)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    setDeliveryCostOpen(true);
                  }
                }}
                className={actionCardClass}
              >
                Money out for a delivery run
              </Card>

              <Card
                role="button"
                tabIndex={0}
                onClick={() => setMilesOpen(true)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    setMilesOpen(true);
                  }
                }}
                className={actionCardClass}
              >
                Log miles
              </Card>

              {/* Paid orders info — zero taps; after action rows, before secondary */}
              {newPaidCount > 0 && (
                <p className="text-base text-muted-foreground">
                  {newPaidCount === 1
                    ? "1 new paid order — capacity already set aside"
                    : `${newPaidCount} new paid orders — capacity already set aside`}
                </p>
              )}

              {/* Cost capture lives inside the work — overlay, not a destination. */}
              <button
                type="button"
                onClick={() => setMoneyLeftOpen(true)}
                className="flex min-h-14 items-center justify-center rounded-xl border px-4 text-base font-medium transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
              >
                Money just left
              </button>

              {view.activeTrayCount > 0 && (
                <button
                  type="button"
                  onClick={() => setRecountOpen(true)}
                  className="flex min-h-11 items-center text-left text-sm text-muted-foreground underline-offset-4 hover:underline focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                >
                  Count the shelf
                </button>
              )}

              {/* TODO(design-debt): Today now carries five secondary controls — Count the shelf,
                  Sell online, Seed rates, Equipment, and the backup line. A sixth breaks the
                  pattern and needs a rethink, not another row. */}
              {view.activeTrayCount > 0 && (
                <button
                  type="button"
                  onClick={() => setSellOpen(true)}
                  className="flex min-h-11 items-center text-left text-sm text-muted-foreground underline-offset-4 hover:underline focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                >
                  Sell online
                </button>
              )}

              <button
                type="button"
                onClick={() => setSeedRatesOpen(true)}
                className="flex min-h-11 items-center text-left text-sm text-muted-foreground underline-offset-4 hover:underline focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
              >
                Seed rates
              </button>

              <button
                type="button"
                onClick={() => setEquipmentOpen(true)}
                className="flex min-h-11 items-center text-left text-sm text-muted-foreground underline-offset-4 hover:underline focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
              >
                Equipment
              </button>

              <button
                type="button"
                onClick={() => setBackupOpen(true)}
                className="text-left text-sm text-muted-foreground underline-offset-4 hover:underline focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
              >
                {backupLine}
              </button>
            </div>
          )}
        </>
      )}

      {/* TODO(stage-5:enforced-numbers): wire 60s-to-first-tray, 12-taps-per-day and
          6-taps-link-to-paid as tests that fail the build. */}
      <SowSheet
        open={sheetOpen}
        onOpenChange={setSheetOpen}
        crops={crops}
        onSow={handleSow}
      />

      <WeightPad
        open={harvestOpen}
        onOpenChange={handleHarvestOpenChange}
        groups={harvestGroupsForPad ?? view?.harvests ?? []}
        onDone={handleHarvestDone}
        onDiscarded={handleDiscarded}
      />

      <RecountSheet
        open={recountOpen}
        onOpenChange={setRecountOpen}
        onDone={handleRecountDone}
      />

      <FarmBackupSheet
        open={backupOpen}
        onOpenChange={setBackupOpen}
        onRestored={handleRestored}
      />

      <SellOnlineSheet open={sellOpen} onOpenChange={setSellOpen} />

      <SeedRatesSheet
        open={seedRatesOpen}
        onOpenChange={setSeedRatesOpen}
        onSaved={() => {
          void listCrops().then(setCrops).catch(console.error);
        }}
      />

      <MoneyJustLeftSheet
        open={moneyLeftOpen}
        onOpenChange={setMoneyLeftOpen}
        moment="money_just_left"
      />

      <MoneyJustLeftSheet
        open={deliveryCostOpen}
        onOpenChange={setDeliveryCostOpen}
        moment="delivery"
      />

      <MilesSheet open={milesOpen} onOpenChange={setMilesOpen} />

      <EquipmentSheet
        open={equipmentOpen}
        onOpenChange={setEquipmentOpen}
      />
    </main>
  );
}
