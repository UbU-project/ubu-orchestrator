# Codegen

Generated artifacts must be reproducible from committed source and pinned tool
versions. Generated OpenAPI output lives at `openapi/openapi.generated.json`.

Codegen changes should include:

- The generator command.
- The source contract change that required regeneration.
- A review note when generated output changes public API shape.
