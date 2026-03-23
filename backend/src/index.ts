import cors from "cors";
import dotenv from "dotenv";
import express from "express";
import fs from "fs";
import path from "path";
import swaggerUi from "swagger-ui-express";
import YAML from "yamljs";

dotenv.config();

const app = express();
const port = Number(process.env.PORT || 4000);

app.use(cors());
app.use(express.json());

const docsDir = path.join(__dirname, "docs");
const openapiYamlPath = path.join(docsDir, "openapi.yaml");
const openapiJsonPath = path.join(docsDir, "openapi.json");

let openapiSpec: unknown = null;
try {
  openapiSpec = YAML.load(openapiYamlPath);
} catch (error) {
  console.warn("OpenAPI spec could not be loaded:", error);
}

if (process.env.NODE_ENV !== "production" && openapiSpec) {
  try {
    fs.writeFileSync(openapiJsonPath, JSON.stringify(openapiSpec, null, 2));
  } catch (error) {
    console.warn("OpenAPI spec could not be exported:", error);
  }

  app.use("/api/docs", swaggerUi.serve, swaggerUi.setup(openapiSpec));
  app.get("/api/docs/openapi.json", (_req, res) => {
    res.json(openapiSpec);
  });
}

app.get("/health", (_req, res) => {
  res.status(200).json({
    status: "ok",
    service: "amana-backend",
    timestamp: new Date().toISOString(),
  });
});

app.listen(port, () => {
  console.log(`Amana backend listening on port ${port}`);
});
