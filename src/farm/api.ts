import { invoke } from "@tauri-apps/api/core";
import type {
  Asset,
  AttentionItem,
  CapacityRow,
  CostCategory,
  CostEvent,
  CostPerTrayOutcome,
  Crop,
  ExportResult,
  FarmLocation,
  ImportPlan,
  ImportResult,
  HarvestGroup,
  HarvestInput,
  IncomeCategory,
  IncomeRecord,
  MileageTrip,
  MoneyStatus,
  NewPaidOrders,
  OfferView,
  OrderView,
  PollResult,
  ReconciliationDate,
  ShopPage,
  StripeAccountPreview,
  RecountCrop,
  RecountEntry,
  RecountResult,
  ResolveResult,
  SnapshotInfo,
  TodayView,
  TrayView,
  UndoResult,
} from "./types";

export function listCrops(): Promise<Crop[]> {
  return invoke("list_crops");
}

export function updateCropSeedRate(
  cropId: string,
  seedRateOzPerTray: number | null,
): Promise<Crop> {
  return invoke("update_crop_seed_rate", {
    cropId,
    seedRateOzPerTray,
  });
}

export function listTrays(): Promise<TrayView[]> {
  return invoke("list_trays");
}

export function todayView(): Promise<TodayView> {
  return invoke("today_view");
}

export function sowTray(
  cropId: string,
  quantity: number,
  seedOz?: number | null,
): Promise<TrayView> {
  return invoke("sow_tray", { cropId, quantity, seedOz: seedOz ?? null });
}

export function advanceTray(trayId: string): Promise<TrayView> {
  return invoke("advance_tray", { trayId });
}

export function advanceTrays(trayIds: string[]): Promise<void> {
  return invoke("advance_trays", { trayIds });
}

export function harvestTray(
  trayId: string,
  actualYieldOz: number,
): Promise<TrayView> {
  return invoke("harvest_tray", { trayId, actualYieldOz });
}

export function harvestTrays(
  trayIds: string[],
  actualYieldOz: number,
): Promise<void> {
  return invoke("harvest_trays", { trayIds, actualYieldOz });
}

export function harvestGroups(groups: HarvestInput[]): Promise<void> {
  return invoke("harvest_groups", { groups });
}

export function discardTray(trayId: string): Promise<TrayView> {
  return invoke("discard_tray", { trayId });
}

export function discardFromGroup(
  trayIds: string[],
  quantity: number,
): Promise<HarvestGroup | null> {
  return invoke("discard_from_group", { trayIds, quantity });
}

export function undoLast(): Promise<UndoResult | null> {
  return invoke("undo_last");
}

export function capacityByHarvestDate(): Promise<CapacityRow[]> {
  return invoke("capacity_by_harvest_date");
}

export function moneyStatus(): Promise<MoneyStatus> {
  return invoke("money_status");
}

export function listOrders(harvestDate?: string | null): Promise<OrderView[]> {
  return invoke("list_orders", { harvestDate: harvestDate ?? null });
}

export function previewStripeKey(key: string): Promise<StripeAccountPreview> {
  return invoke("preview_stripe_key", { key });
}

export function confirmStripeKey(key: string): Promise<MoneyStatus> {
  return invoke("confirm_stripe_key", { key });
}

export function setCheckoutEndpointUrl(url: string): Promise<MoneyStatus> {
  return invoke("set_checkout_endpoint_url", { url });
}

export function listOffers(harvestDate: string): Promise<OfferView[]> {
  return invoke("list_offers", { harvestDate });
}

export function setOffer(
  harvestDate: string,
  cropId: string,
  priceCents: number,
): Promise<OfferView> {
  return invoke("set_offer", { harvestDate, cropId, priceCents });
}

export function removeOffer(offerId: string): Promise<void> {
  return invoke("remove_offer", { offerId });
}

export function generateShopPage(): Promise<ShopPage> {
  return invoke("generate_shop_page");
}

export function openShopPageFolder(): Promise<void> {
  return invoke("open_shop_page_folder");
}

export function pollStripe(): Promise<PollResult> {
  return invoke("poll_stripe");
}

export function takeNewPaidOrders(): Promise<NewPaidOrders> {
  return invoke("take_new_paid_orders");
}

export function reconciliation(): Promise<ReconciliationDate[]> {
  return invoke("reconciliation");
}

/** Debug-only. Absent from release builds. */
export function devBackdateTray(trayId: string, days: number): Promise<void> {
  return invoke("dev_backdate_tray", { trayId, days });
}

export function listSnapshots(): Promise<SnapshotInfo[]> {
  return invoke("list_snapshots");
}

export function takeSnapshot(): Promise<SnapshotInfo> {
  return invoke("take_snapshot");
}

export function restoreSnapshot(path: string): Promise<void> {
  return invoke("restore_snapshot", { path });
}

export function farmLocation(): Promise<FarmLocation> {
  return invoke("farm_location");
}

export function openFarmFolder(): Promise<void> {
  return invoke("open_farm_folder");
}

export function exportBundle(): Promise<ExportResult> {
  return invoke("export_bundle");
}

export function openExportFolder(path: string): Promise<void> {
  return invoke("open_export_folder", { path });
}

export function previewImport(bundlePath: string): Promise<ImportPlan> {
  return invoke("preview_import", { bundlePath });
}

export function applyImport(bundlePath: string): Promise<ImportResult> {
  return invoke("apply_import", { bundlePath });
}

export function recountState(): Promise<RecountCrop[]> {
  return invoke("recount_state");
}

export function applyRecount(entries: RecountEntry[]): Promise<RecountResult> {
  return invoke("apply_recount", { entries });
}

export function checkAttention(): Promise<AttentionItem[]> {
  return invoke("check_attention");
}

export function resolveAttention(
  id: string,
  action: string,
): Promise<ResolveResult> {
  return invoke("resolve_attention", { id, action });
}

export function dismissAttention(id: string): Promise<void> {
  return invoke("dismiss_attention", { id });
}

export function listCostCategories(): Promise<CostCategory[]> {
  return invoke("list_cost_categories");
}

export function receiptSourceInfo(path: string): Promise<{
  fileName: string;
  sizeBytes: number;
}> {
  return invoke("receipt_source_info", { path });
}

export function recordCost(input: {
  amountCents: number;
  payee: string;
  categoryId: string;
  datePaid: string;
  descriptor?: string | null;
  receiptSourcePath?: string | null;
}): Promise<CostEvent> {
  return invoke("record_cost", {
    amountCents: input.amountCents,
    payee: input.payee,
    categoryId: input.categoryId,
    datePaid: input.datePaid,
    descriptor: input.descriptor ?? null,
    receiptSourcePath: input.receiptSourcePath ?? null,
  });
}

export function listMileageTrips(): Promise<MileageTrip[]> {
  return invoke("list_mileage_trips");
}

export function recordMileageTrip(input: {
  tripDate: string;
  miles: number;
  purpose?: string | null;
}): Promise<MileageTrip> {
  return invoke("record_mileage_trip", {
    tripDate: input.tripDate,
    miles: input.miles,
    purpose: input.purpose ?? null,
  });
}

export function correctMileageTrip(input: {
  tripId: string;
  tripDate: string;
  miles: number;
  purpose?: string | null;
}): Promise<MileageTrip> {
  return invoke("correct_mileage_trip", {
    tripId: input.tripId,
    tripDate: input.tripDate,
    miles: input.miles,
    purpose: input.purpose ?? null,
  });
}

export function voidMileageTrip(tripId: string): Promise<void> {
  return invoke("void_mileage_trip", { tripId });
}

export function listAssets(): Promise<Asset[]> {
  return invoke("list_assets");
}

export function recordAsset(input: {
  description: string;
  placedInServiceOn: string;
  costCents: number;
  disposalDate?: string | null;
}): Promise<Asset> {
  return invoke("record_asset", {
    description: input.description,
    placedInServiceOn: input.placedInServiceOn,
    costCents: input.costCents,
    disposalDate: input.disposalDate ?? null,
  });
}

export function correctAsset(input: {
  assetId: string;
  description: string;
  placedInServiceOn: string;
  costCents: number;
  disposalDate?: string | null;
}): Promise<Asset> {
  return invoke("correct_asset", {
    assetId: input.assetId,
    description: input.description,
    placedInServiceOn: input.placedInServiceOn,
    costCents: input.costCents,
    disposalDate: input.disposalDate ?? null,
  });
}

export function voidAsset(assetId: string): Promise<void> {
  return invoke("void_asset", { assetId });
}

export function listIncomeCategories(): Promise<IncomeCategory[]> {
  return invoke("list_income_categories");
}

export function listIncome(): Promise<IncomeRecord[]> {
  return invoke("list_income");
}

export function recordIncome(input: {
  amountCents: number;
  source: string;
  categoryId: string;
  dateReceived: string;
  descriptor?: string | null;
  receiptSourcePath?: string | null;
}): Promise<IncomeRecord> {
  return invoke("record_income", {
    amountCents: input.amountCents,
    source: input.source,
    categoryId: input.categoryId,
    dateReceived: input.dateReceived,
    descriptor: input.descriptor ?? null,
    receiptSourcePath: input.receiptSourcePath ?? null,
  });
}

export function correctIncome(input: {
  incomeId: string;
  amountCents: number;
  source: string;
  categoryId: string;
  dateReceived: string;
  descriptor?: string | null;
  receiptSourcePath?: string | null;
}): Promise<IncomeRecord> {
  return invoke("correct_income", {
    incomeId: input.incomeId,
    amountCents: input.amountCents,
    source: input.source,
    categoryId: input.categoryId,
    dateReceived: input.dateReceived,
    descriptor: input.descriptor ?? null,
    receiptSourcePath: input.receiptSourcePath ?? null,
  });
}

export function voidIncome(incomeId: string): Promise<void> {
  return invoke("void_income", { incomeId });
}

export function costPerTray(input: {
  window: string;
  from?: string | null;
  to?: string | null;
  categoryIds?: string[] | null;
}): Promise<CostPerTrayOutcome> {
  return invoke("cost_per_tray", {
    window: input.window,
    from: input.from ?? null,
    to: input.to ?? null,
    categoryIds: input.categoryIds ?? null,
  });
}
