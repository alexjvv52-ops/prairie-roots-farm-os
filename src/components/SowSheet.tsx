import { useEffect, useState } from "react";
import type { Crop } from "@/farm/types";
import { addDays, localToday, readyLabel } from "@/farm/dates";
import {
  confirmSeedQuantity,
  freshSeedProposal,
  onOperatorSeedEdit,
  onProposalInputsChanged,
  type SeedFieldState,
} from "@/farm/seedPrefill";
import {
  Sheet,
  SheetContent,
  SheetTitle,
} from "@/components/ui/sheet";
import { Button } from "@/components/ui/button";
import { MoneyJustLeftSheet } from "@/components/MoneyJustLeftSheet";

type SowSheetProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  crops: Crop[];
  onSow: (crop: Crop, quantity: number, seedOz: number | null) => void;
};

export function SowSheet({ open, onOpenChange, crops, onSow }: SowSheetProps) {
  const [selectedCrop, setSelectedCrop] = useState<Crop | null>(null);
  const [quantity, setQuantity] = useState(1);
  const [seedField, setSeedField] = useState<SeedFieldState>({
    value: "",
    dirty: false,
  });
  const [seedError, setSeedError] = useState<string | null>(null);
  const [costOpen, setCostOpen] = useState(false);

  function reset() {
    setSelectedCrop(null);
    setQuantity(1);
    setSeedField({ value: "", dirty: false });
    setSeedError(null);
  }

  function handleOpenChange(next: boolean) {
    if (!next && costOpen) return;
    if (!next) reset();
    onOpenChange(next);
  }

  function pickCrop(crop: Crop) {
    setQuantity(1);
    setSelectedCrop(crop);
    setSeedField(freshSeedProposal(crop.seedRateOzPerTray, 1));
    setSeedError(null);
  }

  useEffect(() => {
    if (!selectedCrop) return;
    setSeedField((prev) =>
      onProposalInputsChanged(
        prev,
        selectedCrop.seedRateOzPerTray,
        quantity,
      ),
    );
  }, [quantity, selectedCrop]);

  function confirmSow() {
    if (!selectedCrop) return;
    const parsed = confirmSeedQuantity(seedField);
    if (!parsed.ok) {
      setSeedError(parsed.error);
      return;
    }
    setSeedError(null);
    onSow(selectedCrop, quantity, parsed.quantity);
    reset();
  }

  return (
    <>
      <Sheet open={open} onOpenChange={handleOpenChange}>
        <SheetContent
          side="bottom"
          showCloseButton={false}
          aria-describedby={undefined}
        >
          <div className="mx-auto w-full max-w-md p-4">
            {selectedCrop === null ? (
              <>
                <SheetTitle className="sr-only">Pick a crop to sow</SheetTitle>
                <div className="grid grid-cols-2 gap-3">
                  {crops.map((crop) => (
                    <button
                      key={crop.id}
                      type="button"
                      onClick={() => pickCrop(crop)}
                      className="flex min-h-24 flex-col items-start justify-center gap-1 rounded-xl border bg-card p-4 text-left transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                    >
                      <span className="text-lg font-medium leading-tight">
                        {crop.name}
                      </span>
                      <span className="text-sm text-muted-foreground">
                        ~{crop.growthDays} days
                      </span>
                    </button>
                  ))}
                </div>
                <button
                  type="button"
                  onClick={() => setCostOpen(true)}
                  className="mt-4 flex min-h-11 w-full items-center justify-center rounded-xl border px-4 text-sm font-medium transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                >
                  Money just left
                </button>
              </>
            ) : (
              <>
                <SheetTitle className="sr-only">
                  Confirm sowing {selectedCrop.name}
                </SheetTitle>
                <div className="flex flex-col gap-6">
                  <div>
                    <Button
                      type="button"
                      variant="ghost"
                      onClick={() => setSelectedCrop(null)}
                      className="h-14 -ml-2 px-3 text-base text-muted-foreground"
                    >
                      ← Back
                    </Button>
                  </div>

                  <h2 className="text-2xl font-semibold">{selectedCrop.name}</h2>

                  <div className="flex items-center justify-center gap-6">
                    <button
                      type="button"
                      aria-label="Fewer trays"
                      onClick={() => setQuantity((q) => Math.max(1, q - 1))}
                      disabled={quantity <= 1}
                      className="flex size-14 items-center justify-center rounded-full border text-2xl leading-none transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none disabled:opacity-40"
                    >
                      −
                    </button>
                    <span className="w-12 text-center text-3xl font-semibold tabular-nums">
                      {quantity}
                    </span>
                    <button
                      type="button"
                      aria-label="More trays"
                      onClick={() => setQuantity((q) => q + 1)}
                      className="flex size-14 items-center justify-center rounded-full border text-2xl leading-none transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                    >
                      +
                    </button>
                  </div>

                  <p className="text-center text-base text-muted-foreground">
                    {readyLabel(
                      addDays(localToday(), selectedCrop.growthDays),
                    )}
                  </p>

                  <label className="flex flex-col gap-2">
                    <span className="text-sm text-muted-foreground">
                      Seed weight (oz)
                    </span>
                    <input
                      type="text"
                      inputMode="decimal"
                      value={seedField.value}
                      onChange={(e) => {
                        setSeedError(null);
                        setSeedField((prev) =>
                          onOperatorSeedEdit(prev, e.target.value),
                        );
                      }}
                      placeholder="Weigh and enter"
                      className="h-14 w-full rounded-xl border bg-background px-4 text-lg tabular-nums focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                    />
                    {seedError ? (
                      <span className="text-sm text-destructive">{seedError}</span>
                    ) : null}
                  </label>

                  <Button
                    type="button"
                    onClick={confirmSow}
                    className="h-14 w-full text-lg"
                  >
                    Sow
                  </Button>

                  <button
                    type="button"
                    onClick={() => setCostOpen(true)}
                    className="flex min-h-11 w-full items-center justify-center rounded-xl border px-4 text-sm font-medium transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                  >
                    Money just left
                  </button>
                </div>
              </>
            )}
          </div>
        </SheetContent>
      </Sheet>

      <MoneyJustLeftSheet
        open={costOpen}
        onOpenChange={setCostOpen}
        moment="sow"
        stacked
      />
    </>
  );
}
