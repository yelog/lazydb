local M = {}

local defaults = {
  executable = "lazydb",
  cwd = nil,
  config = nil,
  profile = nil,
  read_only = false,
  mouse = nil,
  color = nil,
  icons = nil,
  window = {
    width = 0.85,
    height = 0.80,
    border = "rounded",
    zindex = 50,
  },
}

local values = vim.deepcopy(defaults)

local function optional_string(name, value)
  if value ~= nil and (type(value) ~= "string" or value == "") then
    error("lazydb.nvim: " .. name .. " must be a non-empty string", 3)
  end
end

local function validate(options)
  if type(options) ~= "table" then
    error("lazydb.nvim: setup options must be a table", 3)
  end

  optional_string("executable", options.executable)
  optional_string("config", options.config)
  optional_string("profile", options.profile)

  if options.cwd ~= nil and type(options.cwd) ~= "string" and type(options.cwd) ~= "function" then
    error("lazydb.nvim: cwd must be a string or function", 3)
  end
  if options.cwd == "" then
    error("lazydb.nvim: cwd must not be empty", 3)
  end
  if options.read_only ~= nil and type(options.read_only) ~= "boolean" then
    error("lazydb.nvim: read_only must be a boolean", 3)
  end
  if options.mouse ~= nil and not vim.tbl_contains({ "auto", "on", "off" }, options.mouse) then
    error("lazydb.nvim: mouse must be 'auto', 'on', or 'off'", 3)
  end
  if options.color ~= nil and not vim.tbl_contains({ "auto", "always", "never" }, options.color) then
    error("lazydb.nvim: color must be 'auto', 'always', or 'never'", 3)
  end
  if options.icons ~= nil
      and not vim.tbl_contains({ "nerd-font", "unicode", "ascii" }, options.icons) then
    error("lazydb.nvim: icons must be 'nerd-font', 'unicode', or 'ascii'", 3)
  end
  if options.window ~= nil and type(options.window) ~= "table" then
    error("lazydb.nvim: window must be a table", 3)
  end

  local window = options.window or {}
  for _, name in ipairs({ "width", "height", "zindex" }) do
    if window[name] ~= nil and type(window[name]) ~= "number" then
      error("lazydb.nvim: window." .. name .. " must be a number", 3)
    end
  end
end

function M.setup(options)
  options = options or {}
  validate(options)
  values = vim.tbl_deep_extend("force", vim.deepcopy(defaults), options)
  return vim.deepcopy(values)
end

function M.get()
  return vim.deepcopy(values)
end

function M.argv()
  local argv = { values.executable }

  if values.config then
    vim.list_extend(argv, { "--config", values.config })
  end
  if values.profile then
    vim.list_extend(argv, { "--profile", values.profile })
  end
  if values.read_only then
    argv[#argv + 1] = "--read-only"
  end
  if values.mouse then
    vim.list_extend(argv, { "--mouse", values.mouse })
  end
  if values.color then
    vim.list_extend(argv, { "--color", values.color })
  end
  if values.icons then
    vim.list_extend(argv, { "--icons", values.icons })
  end

  return argv
end

function M.cwd()
  local cwd = values.cwd
  if type(cwd) == "function" then
    cwd = cwd()
  end
  if cwd == nil then
    cwd = vim.fn.getcwd()
  end
  if type(cwd) ~= "string" or cwd == "" then
    error("lazydb.nvim: cwd must resolve to a non-empty string")
  end
  return cwd
end

function M._reset()
  values = vim.deepcopy(defaults)
end

return M
