import { useEffect, useState } from "react";
import type { Crop } from "@/farm/types";
import { listCrops, updateCropSeedRate } from "@/farm/api";
import {
  Sheet,
  SheetContent,
  SheetTitle,
} from "@/components/ui/sheet";
import { Button } from "@/components/ui/button";

type SeedRatesSheetProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSaved?: () => void;
};

function errMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  return "Could not save. Try again.";
}

function formatRate(rate: number | null): string {
  if (rate == null) return "—";
  return `${rate} oz/tray`;
}

/** Blank → null. Non-blank → number (Rust owns validation). */
export function rateFieldToValue(raw: string): number | null {
  const trimmed = raw.trim();
  if (trimmed === "") return null;
  return Number(trimmed);
}

export function SeedRatesSheet({
  open,
  onOpenChange,
  onSaved,
}: SeedRatesSheetProps) {
  const [crops, setCrops] = useState<Crop[]>([]);
  const [editing, setEditing] = useState<Crop | null>(null);
  const [draft, setDraft] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function load() {
    try {
      const rows = await listCrops();
      setCrops(rows);
    } catch (err) {
      console.error(err);
    }
  }

  useEffect(() => {
    if (!open) return;
    setEditing(null);
    setDraft("");
    setError(null);
    void load();
  }, [open]);

  function handleOpenChange(next: boolean) {
    if (!next) {
      setEditing(null);
      setDraft("");
      setError(null);
    }
    onOpenChange(next);
  }

  function startEdit(crop: Crop) {
    setEditing(crop);
    setDraft(
      crop.seedRateOzPerTray == null ? "" : String(crop.seedRateOzPerTray),
    );
    setError(null);
  }

  async function handleSave() {
    if (!editing || saving) return;
    const value = rateFieldToValue(draft);
    // JSON cannot carry NaN/Infinity; those become null on the wire.
    if (value !== null && !Number.isFinite(value)) {
      setError("seed_rate_oz_per_tray must be a finite number");
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await updateCropSeedRate(editing.id, value);
      await load();
      setEditing(null);
      setDraft("");
      onSaved?.();
    } catch (err) {
      setError(errMessage(err));
    } finally {
      setSaving(false);
    }
  }

  return (
    <Sheet open={open} onOpenChange={handleOpenChange}>
      <SheetContent
        side="bottom"
        showCloseButton={false}
        aria-describedby={undefined}
      >
        <div className="mx-auto flex w-full max-w-md flex-col gap-6 p-4 pb-8">
          {editing ? (
            <>
              <div>
                <Button
                  type="button"
                  variant="ghost"
                  onClick={() => {
                    setEditing(null);
                    setError(null);
                  }}
                  className="h-14 -ml-2 px-3 text-base text-muted-foreground"
                >
                  ← Back
                </Button>
              </div>
              <SheetTitle className="text-center text-xl font-semibold">
                {editing.name}
              </SheetTitle>
              <p className="text-center text-base text-muted-foreground">
                Seed rate (oz per tray). Leave blank for no proposal.
              </p>
              <input
                type="text"
                inputMode="decimal"
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
                aria-label="Seed rate ounces per tray"
                className="h-14 w-full rounded-xl border bg-background px-4 text-center text-2xl tabular-nums focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
              />
              {error && (
                <p className="text-center text-sm text-destructive">{error}</p>
              )}
              <Button
                type="button"
                onClick={() => void handleSave()}
                disabled={saving}
                className="h-14 w-full text-lg"
              >
                Save
              </Button>
            </>
          ) : (
            <>
              <SheetTitle className="text-center text-xl font-semibold">
                Seed rates
              </SheetTitle>
              <p className="text-center text-base text-muted-foreground">
                Oz of seed per tray — used to pre-fill at sow.
              </p>
              <ul className="flex flex-col gap-2">
                {crops.map((crop) => (
                  <li key={crop.id}>
                    <button
                      type="button"
                      onClick={() => startEdit(crop)}
                      className="flex min-h-14 w-full items-center justify-between gap-4 rounded-xl border px-4 text-left text-base transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                    >
                      <span className="font-medium">{crop.name}</span>
                      <span className="tabular-nums text-muted-foreground">
                        {formatRate(crop.seedRateOzPerTray)}
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            </>
          )}
        </div>
      </SheetContent>
    </Sheet>
  );
}
