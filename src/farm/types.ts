export type Crop = {
  id: string;
  name: string;
  growthDays: number;
  blackoutDays: number;
  expectedYieldOz: number;
  sortOrder: number;
  /** Oz of seed per tray. null = no pre-fill proposal. */
  seedRateOzPerTray: number | null;
};

export type TrayView = {
  id: string;
  cropId: string;
  cropName: string;
  state: string;
  quantity: number;
  growthDaysAtSow: number | null;
  blackoutDaysAtSow: number | null;
  plannedOn: string | null;
  sownOn: string | null;
  blackoutOn: string | null;
  lightOn: string | null;
  harvestedOn: string | null;
  discardedOn: string | null;
  actualYieldOz: number | null;
  expectedHarvestDate: string | null;
  coverCheckDate: string | null;
  createdAt: string;
  updatedAt: string;
};

export type CapacityRow = {
  harvestDate: string;
  trays: number;
  expectedYieldOz: number;
  soldTrays: number;
  remainingTrays: number;
};

export type MoneyStatus = {
  configured: boolean;
  mode: string | null;
  accountName: string | null;
  lastPollOk: string | null;
  lastPollErr: string | null;
  openOrderCount: number;
  checkoutEndpointUrl: string | null;
};

export type StripeAccountPreview = {
  accountId: string;
  accountName: string;
  mode: string;
};

export type OfferView = {
  id: string | null;
  harvestDate: string;
  cropId: string;
  cropName: string;
  priceCents: number | null;
  stripePriceId: string | null;
  stripeLinkUrl: string | null;
  available: number;
  sold: number;
  remaining: number;
};

export type ShopPage = {
  filePath: string;
  sizeBytes: number;
  generatedAt: string;
  harvestDates: string[];
};

export type OrderView = {
  id: string;
  stripeSessionId: string;
  stripePaymentIntent: string | null;
  harvestDate: string;
  cropId: string;
  cropName: string;
  quantity: number;
  amountCents: number;
  currency: string;
  customerEmail: string | null;
  state: string;
  capacityConsumed: number;
  clientReference: string | null;
  paidAt: string;
  createdAt: string;
  updatedAt: string;
};

export type UndoResult = {
  undoesSeq: number;
  undoneKind: string;
};

export type MoveToLight = {
  trayIds: string[];
  trayCount: number;
};

export type HarvestGroup = {
  cropId: string;
  cropName: string;
  trayIds: string[];
  trayCount: number;
  estimatedYieldOz: number;
};

export type HarvestSummary = {
  trayCount: number;
  varietyCount: number;
  estimatedYieldOz: number;
  singleCropName: string | null;
};

export type HarvestInput = {
  trayIds: string[];
  actualYieldOz: number;
};

export type NextEvent = {
  kind: "light" | "harvest" | string;
  date: string;
  trayCount: number;
  cropName: string;
};

export type TodayView = {
  moveToLight: MoveToLight | null;
  harvests: HarvestGroup[];
  harvestSummary: HarvestSummary | null;
  nextEvent: NextEvent | null;
  activeTrayCount: number;
  /** Distinguishes Script A idle copy (B) from the generic next-event line (C). */
  sownToday: boolean;
};

/** Operator-facing cost category — no tax line numbers. */
export type CostCategory = {
  id: string;
  name: string;
  descriptorRequired: boolean;
};

export type CostEvent = {
  eventId: string;
  origin: string;
  datePaid: string;
  amountCents: number;
  payee: string;
  canonicalCategory: string;
  descriptor: string;
  receiptFileRef: string | null;
  createdAt: string;
  updatedAt: string;
};

/** One dated trip, stored in miles. There is no dollar value on this type. */
export type MileageTrip = {
  tripId: string;
  origin: string;
  tripDate: string;
  miles: number;
  purpose: string | null;
  lastEventId: string;
  createdAt: string;
  updatedAt: string;
};

/** Asset register row — four operator fields. Nothing computed. */
export type Asset = {
  assetId: string;
  origin: string;
  description: string;
  placedInServiceOn: string;
  costCents: number;
  disposalDate: string | null;
  lastEventId: string;
  createdAt: string;
  updatedAt: string;
};

export type IncludedPayment = {
  eventId: string;
  datePaid: string;
  payee: string;
  canonicalCategory: string;
  amountCents: number;
};

export type IncludedTrayRecord = {
  eventId: string;
  occurredOn: string;
  varietyOrItem: string;
  quantity: number;
  seedQuantityRecorded: boolean;
};

export type MethodStatement = {
  windowLabel: string;
  windowFrom: string;
  windowTo: string;
  originFilter: string;
  paymentRule: string;
  physicalRule: string;
  joinRule: string;
  exclusionRule: string;
  payments: IncludedPayment[];
  trayRecords: IncludedTrayRecord[];
  paymentCount: number;
  trayRecordCount: number;
  totalPaidCents: number;
  totalTrays: number;
  trayRecordsWithSeedRecorded: number;
  trayRecordsWithoutSeedRecorded: number;
  completenessNote: string;
};

/** Derived at query time. Never stored.
 *  Do not cache this in component state beyond the current view. */
export type CostPerTrayFigure = {
  totalPaidCents: number;
  totalTrays: number;
  centsPerTray: number;
};

export type CostPerTrayOutcome =
  | { kind: "computed"; figure: CostPerTrayFigure; method: MethodStatement }
  | { kind: "refused"; reason: string; method: MethodStatement };

export type SnapshotInfo = {
  fileName: string;
  path: string;
  takenAt: string;
  sizeBytes: number;
};

export type FarmLocation = {
  farmDbPath: string;
  folderPath: string;
  lastSnapshotAt: string | null;
};

export type ExportResult = {
  bundlePath: string;
  fileCount: number;
  totalBytes: number;
  exportedAt: string;
};

export type ImportRefusal =
  | { kind: "missingEventId"; lineNo: number }
  | {
      kind: "farmOsConflict";
      eventId: string;
      field: string;
      inThisFarm: string;
      inTheBundle: string;
    }
  | { kind: "commercialClaimingFarmOs"; eventId: string; detail: string }
  | {
      kind: "differentFarm";
      farmRecordsHere: number;
      eventsInBundle: number;
    }
  | { kind: "logVersusDatabase"; detail: string }
  | { kind: "manifestMismatch"; path: string; detail: string }
  | { kind: "schemaVersion"; bundle: number; thisApp: number }
  | { kind: "malformed"; lineNo: number; detail: string };

export type ImportPlan = {
  bundlePath: string;
  bundleExportedAt: string;
  eventsInBundle: number;
  sharedEventIds: number;
  alreadyPresentIdentical: number;
  wouldBeAdded: number;
  foreignRecordsInBundle: number;
  refusals: ImportRefusal[];
  canApply: boolean;
  explanations: string[];
};

export type ImportResult = {
  eventsAdded: number;
  eventsSkippedIdentical: number;
  foreignRecordsAdded: number;
};

export type RecountCrop = {
  cropId: string;
  cropName: string;
  appQuantity: number;
  trayIds: string[];
};

export type RecountEntry = {
  cropId: string;
  countedQuantity: number;
};

export type RecountCropChange = {
  cropId: string;
  cropName: string;
  quantity: number;
};

export type RecountResult = {
  adjustedDown: RecountCropChange[];
  adjustedUp: RecountCropChange[];
  unchanged: number;
};

export type AttentionItem = {
  id: string;
  kind: string;
  entityType: string | null;
  entityId: string | null;
  message: string;
  actions: string[];
  createdAt: string;
};

export type ResolveResult = {
  trayIds: string[];
  openUrl?: string | null;
};

export type PollResult = {
  ok: boolean;
  sessionsApplied: number;
  refundsApplied: number;
  disputesApplied: number;
  error: string | null;
};

export type NewPaidOrders = {
  count: number;
};

export type ReconciliationOrder = {
  id: string;
  cropName: string;
  quantity: number;
  state: string;
  capacityConsumed: number;
  amountCents: number;
  paidAt: string;
};

export type ReconciliationDate = {
  harvestDate: string;
  available: number;
  sold: number;
  remaining: number;
  orders: ReconciliationOrder[];
};
