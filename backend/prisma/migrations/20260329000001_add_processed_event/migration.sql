-- Drop old ProcessedLedger table if it exists
DROP TABLE IF EXISTS "ProcessedLedger";

-- Create new ProcessedEvent table
CREATE TABLE "ProcessedEvent" (
  "id"             SERIAL PRIMARY KEY,
  "ledgerSequence" INTEGER NOT NULL,
  "contractId"     VARCHAR(255) NOT NULL,
  "eventId"        VARCHAR(255) NOT NULL,
  "processedAt"    TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  CONSTRAINT "ProcessedEvent_ledgerSequence_contractId_eventId_key"
    UNIQUE ("ledgerSequence", "contractId", "eventId")
);

CREATE INDEX "ProcessedEvent_ledgerSequence_idx" ON "ProcessedEvent"("ledgerSequence");
