import { useEffect, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import type { FarmLocation, SnapshotInfo } from "@/farm/types";
import {
  farmLocation,
  listSnapshots,
  openFarmFolder,
  restoreSnapshot,
} from "@/farm/api";
import { snapshotLabel } from "@/farm/dates";
import {
  Sheet,
  SheetContent,
  SheetTitle,
} from "@/components/ui/sheet";
import { Button } from "@/components/ui/button";

type FarmBackupSheetProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onRestored: (label: string) => void;
};

type ConfirmTarget = {
  path: string;
  label: string;
  /** When true, confirm copy refers to a picked file rather than a dated snapshot. */
  fromFile?: boolean;
};

export function FarmBackupSheet({
  open,
  onOpenChange,
  onRestored,
}: FarmBackupSheetProps) {
  const [location, setLocation] = useState<FarmLocation | null>(null);
  const [snapshots, setSnapshots] = useState<SnapshotInfo[]>([]);
  const [confirm, setConfirm] = useState<ConfirmTarget | null>(null);
  const [restoring, setRestoring] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function load() {
    try {
      const [loc, snaps] = await Promise.all([farmLocation(), listSnapshots()]);
      setLocation(loc);
      setSnapshots(snaps);
      setError(null);
    } catch (err) {
      console.error(err);
    }
  }

  useEffect(() => {
    if (!open) return;
    setConfirm(null);
    setError(null);
    void load();
  }, [open]);

  function handleOpenChange(next: boolean) {
    if (!next) {
      setConfirm(null);
      setError(null);
    }
    onOpenChange(next);
  }

  async function handleRestore() {
    if (!confirm || restoring) return;
    setRestoring(true);
    setError(null);
    try {
      await restoreSnapshot(confirm.path);
      const label = confirm.label;
      setConfirm(null);
      onOpenChange(false);
      onRestored(label);
    } catch (err) {
      const message =
        typeof err === "string"
          ? err
          : err instanceof Error
            ? err.message
            : "Restore failed.";
      setError(message);
      console.error(err);
    } finally {
      setRestoring(false);
    }
  }

  async function handleMovedComputers() {
    setError(null);
    try {
      const selected = await openDialog({
        multiple: false,
        filters: [{ name: "Farm database", extensions: ["db"] }],
      });
      if (selected === null) return;
      const path = Array.isArray(selected) ? selected[0] : selected;
      if (!path) return;
      const name = path.split(/[/\\]/).pop() ?? path;
      const fromName = snapshotLabelFromFileName(name);
      if (fromName) {
        setConfirm({ path, label: fromName });
      } else {
        setConfirm({ path, label: "this file", fromFile: true });
      }
    } catch (err) {
      console.error(err);
      setError("Could not open the file picker.");
    }
  }

  return (
    <Sheet open={open} onOpenChange={handleOpenChange}>
      <SheetContent
        side="bottom"
        showCloseButton={false}
        aria-describedby={undefined}
        className="max-h-[85vh] overflow-y-auto"
      >
        <div className="mx-auto flex w-full max-w-md flex-col gap-6 p-4 pb-8">
          {confirm ? (
            <>
              <SheetTitle className="text-xl font-medium leading-snug">
                {confirm.fromFile
                  ? "Restore the farm from this file?"
                  : `Restore the farm as it was ${confirm.label}?`}
              </SheetTitle>
              <div className="flex flex-col gap-3">
                <Button
                  type="button"
                  className="h-14 text-base"
                  disabled={restoring}
                  onClick={handleRestore}
                >
                  Restore
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  className="h-12 text-base"
                  disabled={restoring}
                  onClick={() => {
                    setConfirm(null);
                    setError(null);
                  }}
                >
                  Back
                </Button>
              </div>
              <p className="text-sm text-muted-foreground">
                Your farm right now will be saved first, so this can be undone.
              </p>
              {error && (
                <p className="text-sm text-destructive">{error}</p>
              )}
            </>
          ) : (
            <>
              <SheetTitle className="text-xl font-medium">
                Where your farm lives
              </SheetTitle>
              <div className="flex flex-col gap-3">
                <p className="break-all font-mono text-sm text-muted-foreground">
                  {location?.folderPath ?? "…"}
                </p>
                <Button
                  type="button"
                  variant="outline"
                  className="h-12 self-start text-base"
                  onClick={() => {
                    void openFarmFolder().catch((err) => {
                      console.error(err);
                    });
                  }}
                >
                  Open folder
                </Button>
              </div>
              <p className="text-sm text-muted-foreground">
                Backups are on this computer. To protect against losing the
                computer itself, copy this folder to a USB stick or a cloud
                folder.
              </p>
              <div className="flex flex-col gap-1">
                {snapshots.map((snap) => {
                  const label = snapshotLabel(new Date(snap.takenAt));
                  return (
                    <button
                      key={snap.path}
                      type="button"
                      onClick={() =>
                        setConfirm({ path: snap.path, label })
                      }
                      className="rounded-md px-2 py-3 text-left text-base transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                    >
                      {label}
                    </button>
                  );
                })}
                {snapshots.length === 0 && (
                  <p className="text-sm text-muted-foreground">
                    No backups yet.
                  </p>
                )}
              </div>
              <button
                type="button"
                onClick={() => void handleMovedComputers()}
                className="mt-2 self-start text-sm text-muted-foreground underline-offset-4 hover:underline focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
              >
                I moved computers
              </button>
              {error && (
                <p className="text-sm text-destructive">{error}</p>
              )}
            </>
          )}
        </div>
      </SheetContent>
    </Sheet>
  );
}

function snapshotLabelFromFileName(name: string): string | null {
  // farm-YYYY-MM-DD-HHMMSS.db
  const m = /^farm-(\d{4})-(\d{2})-(\d{2})-(\d{2})(\d{2})(\d{2})(?:-\d+)?\.db$/.exec(
    name,
  );
  if (!m) return null;
  const [, y, mo, d, h, mi, s] = m;
  const date = new Date(
    Number(y),
    Number(mo) - 1,
    Number(d),
    Number(h),
    Number(mi),
    Number(s),
  );
  return snapshotLabel(date);
}
