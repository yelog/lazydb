local config = require("lazydb.config")

local M = {}

local SUPPORTED_CLI_API = 1

local function check_capabilities(executable)
  local argv = { executable, "capabilities", "--json" }
  local started, process = pcall(vim.system, argv, { text = true })
  if not started then
    vim.health.error("Could not run capabilities check: " .. tostring(process))
    return
  end

  local waited, result = pcall(process.wait, process, 5000)
  if not waited then
    vim.health.error("Capabilities check failed: " .. tostring(result))
    return
  end
  if result.code ~= 0 then
    local detail = result.stderr ~= "" and result.stderr or ("exit code " .. tostring(result.code))
    vim.health.error("`lazydb capabilities --json` failed: " .. vim.trim(detail))
    return
  end

  local decoded, capabilities = pcall(vim.json.decode, result.stdout or "")
  if not decoded or type(capabilities) ~= "table" then
    vim.health.error("`lazydb capabilities --json` returned invalid JSON")
    return
  end
  if capabilities.cli_api ~= SUPPORTED_CLI_API then
    vim.health.error(string.format(
      "Unsupported cli_api %s; lazydb.nvim requires cli_api %d",
      vim.inspect(capabilities.cli_api),
      SUPPORTED_CLI_API
    ))
    return
  end

  vim.health.ok(string.format(
    "LazyDB %s reports compatible cli_api %d",
    capabilities.version or "(unknown version)",
    capabilities.cli_api
  ))
end

local function check_locale()
  local locale = vim.env.LC_ALL
  if not locale or locale == "" then
    locale = vim.env.LC_CTYPE
  end
  if not locale or locale == "" then
    locale = vim.env.LANG
  end

  if not locale or locale == "" then
    vim.health.warn("No locale environment variable is set; use a UTF-8 locale")
  elseif locale:lower():find("utf%-?8") then
    vim.health.ok("UTF-8 locale detected: " .. locale)
  else
    vim.health.warn("Locale does not advertise UTF-8: " .. locale)
  end
end

function M.check()
  vim.health.start("lazydb.nvim")

  if vim.fn.has("nvim-0.10") == 1 then
    vim.health.ok("Neovim 0.10 or newer detected")
  else
    vim.health.error("Neovim 0.10 or newer is required")
  end

  if vim.fn.exists("*jobstart") == 1 then
    vim.health.ok("Terminal jobs are available")
  else
    vim.health.error("jobstart() is unavailable")
  end

  local executable = config.get().executable
  if vim.fn.executable(executable) == 1 then
    vim.health.ok("Executable found: " .. executable)
    check_capabilities(executable)
  else
    vim.health.error("Executable not found or not executable: " .. executable)
  end

  check_locale()

  if vim.fn.has("clipboard") == 1 or type(vim.g.clipboard) == "table" then
    vim.health.ok("Clipboard provider is available")
  else
    vim.health.warn("No clipboard provider detected; system clipboard integration is unavailable")
  end
end

return M
