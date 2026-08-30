local config = require("lazydb.config")

local M = {}

local sessions = {}
local generation = 0
local autocmd_group
local commands_registered = false

local function is_valid_buffer(buffer)
  return buffer ~= nil and vim.api.nvim_buf_is_valid(buffer)
end

local function is_valid_window(window)
  return window ~= nil and vim.api.nvim_win_is_valid(window)
end

local function close_window(session)
  local window = session.win
  session.win = nil
  if is_valid_window(window) then
    pcall(vim.api.nvim_win_close, window, true)
  end
end

local function dispose(session, options)
  options = options or {}
  if session.disposed then
    return
  end

  session.disposed = true
  if sessions[session.tabpage] == session then
    sessions[session.tabpage] = nil
  end

  local job = session.job
  session.job = nil
  session.state = "stopped"
  close_window(session)

  if options.stop_job and job and job > 0 then
    pcall(vim.fn.jobstop, job)
  end
  if options.wipe_buffer ~= false and is_valid_buffer(session.buffer) then
    pcall(vim.api.nvim_buf_delete, session.buffer, { force = true })
  end
end

local function dispose_all()
  local pending = {}
  for _, session in pairs(sessions) do
    pending[#pending + 1] = session
  end
  for _, session in ipairs(pending) do
    dispose(session, { stop_job = true })
  end
end

local function dimension(value, total, maximum)
  local resolved = value
  if value > 0 and value <= 1 then
    resolved = math.floor(total * value)
  else
    resolved = math.floor(value)
  end
  return math.max(1, math.min(resolved, maximum))
end

local function float_config()
  local options = config.get().window
  local columns = vim.o.columns
  local available_lines = math.max(1, vim.o.lines - vim.o.cmdheight)
  local width = dimension(options.width, columns, math.max(1, columns - 2))
  local height = dimension(options.height, available_lines, math.max(1, available_lines - 2))

  return {
    relative = "editor",
    row = math.max(0, math.floor((available_lines - height) / 2)),
    col = math.max(0, math.floor((columns - width) / 2)),
    width = width,
    height = height,
    style = "minimal",
    border = options.border,
    zindex = options.zindex,
  }
end

local function show(session)
  if not is_valid_buffer(session.buffer) then
    return false
  end

  if is_valid_window(session.win) then
    vim.api.nvim_set_current_win(session.win)
  else
    session.win = vim.api.nvim_open_win(session.buffer, true, float_config())
  end
  pcall(vim.cmd, "startinsert")
  return true
end

local function preserve_start_error(session, message)
  session.job = nil
  session.state = "exited"
  session.exit_code = -1

  if not is_valid_buffer(session.buffer) then
    return
  end

  local modifiable = vim.bo[session.buffer].modifiable
  pcall(function()
    vim.bo[session.buffer].modifiable = true
    vim.api.nvim_buf_set_lines(session.buffer, 0, -1, false, {
      "LazyDB failed to start.",
      "",
      tostring(message),
    })
    vim.bo[session.buffer].modifiable = modifiable
  end)
end

local function handle_exit(tabpage, token, job_id, exit_code)
  local session = sessions[tabpage]
  if not session or session.generation ~= token or session.job ~= job_id then
    return
  end

  session.job = nil
  if exit_code == 0 then
    dispose(session, { wipe_buffer = true })
    return
  end

  session.state = "exited"
  session.exit_code = exit_code
end

local function create_session(tabpage)
  generation = generation + 1
  local token = generation
  local buffer = vim.api.nvim_create_buf(false, true)
  local session = {
    tabpage = tabpage,
    buffer = buffer,
    generation = token,
    state = "starting",
  }
  sessions[tabpage] = session

  pcall(vim.api.nvim_buf_set_name, buffer, string.format("lazydb://%d/%d", tabpage, token))
  vim.bo[buffer].bufhidden = "hide"
  vim.bo[buffer].swapfile = false

  show(session)

  local ok, job_or_error = pcall(vim.fn.jobstart, config.argv(), {
    term = true,
    cwd = config.cwd(),
    on_exit = function(job_id, exit_code)
      vim.schedule(function()
        handle_exit(tabpage, token, job_id, exit_code)
      end)
    end,
  })

  if not ok or type(job_or_error) ~= "number" or job_or_error <= 0 then
    preserve_start_error(session, ok and ("jobstart returned " .. tostring(job_or_error)) or job_or_error)
    return session
  end

  session.job = job_or_error
  session.state = "running"
  pcall(vim.cmd, "startinsert")
  return session
end

local function ensure_autocmds()
  if autocmd_group then
    return
  end

  autocmd_group = vim.api.nvim_create_augroup("lazydb.nvim", { clear = true })

  vim.api.nvim_create_autocmd("WinClosed", {
    group = autocmd_group,
    callback = function(args)
      local closed = tonumber(args.match)
      for _, session in pairs(sessions) do
        if session.win == closed then
          session.win = nil
        end
      end
    end,
  })

  vim.api.nvim_create_autocmd("BufWipeout", {
    group = autocmd_group,
    callback = function(args)
      for _, session in pairs(sessions) do
        if session.buffer == args.buf then
          dispose(session, { stop_job = true, wipe_buffer = false })
          return
        end
      end
    end,
  })

  vim.api.nvim_create_autocmd("TabClosed", {
    group = autocmd_group,
    callback = function()
      vim.schedule(function()
        local closed = {}
        for tabpage, session in pairs(sessions) do
          if not vim.api.nvim_tabpage_is_valid(tabpage) then
            closed[#closed + 1] = session
          end
        end
        for _, session in ipairs(closed) do
          dispose(session, { stop_job = true })
        end
      end)
    end,
  })

  vim.api.nvim_create_autocmd("VimResized", {
    group = autocmd_group,
    callback = function()
      for _, session in pairs(sessions) do
        if is_valid_window(session.win) then
          pcall(vim.api.nvim_win_set_config, session.win, float_config())
        end
      end
    end,
  })

  vim.api.nvim_create_autocmd("VimLeavePre", {
    group = autocmd_group,
    callback = dispose_all,
  })
end

function M.setup(options)
  config.setup(options)
  ensure_autocmds()
  return M
end

function M.open()
  ensure_autocmds()
  local tabpage = vim.api.nvim_get_current_tabpage()
  local session = sessions[tabpage]

  if session and is_valid_buffer(session.buffer) then
    show(session)
  else
    if session then
      dispose(session, { stop_job = true, wipe_buffer = false })
    end
    create_session(tabpage)
  end

  return M.status(tabpage)
end

function M.hide()
  local session = sessions[vim.api.nvim_get_current_tabpage()]
  if not session or not is_valid_window(session.win) then
    return false
  end
  close_window(session)
  return true
end

function M.toggle()
  local session = sessions[vim.api.nvim_get_current_tabpage()]
  if session and is_valid_window(session.win) then
    M.hide()
    return M.status()
  end
  return M.open()
end

function M.stop()
  local tabpage = vim.api.nvim_get_current_tabpage()
  local session = sessions[tabpage]
  if not session then
    return false
  end
  dispose(session, { stop_job = true })
  return true
end

function M.restart()
  M.stop()
  return M.open()
end

function M.status(tabpage)
  tabpage = tabpage or vim.api.nvim_get_current_tabpage()
  local session = sessions[tabpage]

  if session and not is_valid_buffer(session.buffer) then
    dispose(session, { stop_job = true, wipe_buffer = false })
    session = nil
  end
  if not session then
    return {
      tabpage = tabpage,
      state = "stopped",
      visible = false,
    }
  end

  if not is_valid_window(session.win) then
    session.win = nil
  end
  return {
    tabpage = tabpage,
    state = session.state,
    visible = session.win ~= nil,
    job_id = session.job,
    buffer = session.buffer,
    window = session.win,
    exit_code = session.exit_code,
  }
end

function M._register_commands()
  if commands_registered then
    return
  end
  commands_registered = true
  ensure_autocmds()

  local commands = {
    LazyDB = { method = "open", description = "Open LazyDB" },
    LazyDBToggle = { method = "toggle", description = "Toggle the LazyDB window" },
    LazyDBHide = { method = "hide", description = "Hide the LazyDB window" },
    LazyDBStop = { method = "stop", description = "Stop LazyDB in this tab" },
    LazyDBRestart = { method = "restart", description = "Restart LazyDB in this tab" },
  }

  for name, command in pairs(commands) do
    vim.api.nvim_create_user_command(name, function()
      require("lazydb")[command.method]()
    end, { desc = command.description })
  end
end

function M._reset()
  dispose_all()
  config._reset()
  if autocmd_group then
    pcall(vim.api.nvim_del_augroup_by_id, autocmd_group)
    autocmd_group = nil
  end
end

return M
