local M = {}

local tests = {}

local function test(name, fn)
  tests[#tests + 1] = { name = name, fn = fn }
end

local function fail(message)
  error(message, 2)
end

local function eq(actual, expected, message)
  if not vim.deep_equal(actual, expected) then
    fail(string.format(
      "%s\nexpected: %s\nactual:   %s",
      message or "values differ",
      vim.inspect(expected),
      vim.inspect(actual)
    ))
  end
end

local function truthy(value, message)
  if not value then
    fail(message or "expected a truthy value")
  end
end

local function wait_for(predicate, message)
  if not vim.wait(500, predicate, 10) then
    fail(message or "condition was not met before timeout")
  end
end

local function unload_plugin()
  local loaded = package.loaded["lazydb"]
  if loaded and loaded._reset then
    pcall(loaded._reset)
  end

  package.loaded["lazydb"] = nil
  package.loaded["lazydb.init"] = nil
  package.loaded["lazydb.config"] = nil
  package.loaded["lazydb.health"] = nil
end

local function fresh_plugin()
  unload_plugin()
  return require("lazydb")
end

local function with_fake_jobs(fn)
  local original_jobstart = vim.fn.jobstart
  local original_jobstop = vim.fn.jobstop
  local fake = {
    next_id = 100,
    starts = {},
    stopped = {},
  }

  vim.fn.jobstart = function(argv, opts)
    local id = fake.next_id
    fake.next_id = fake.next_id + 1
    fake.starts[#fake.starts + 1] = {
      id = id,
      argv = vim.deepcopy(argv),
      opts = opts,
      buffer = vim.api.nvim_get_current_buf(),
    }
    return id
  end

  vim.fn.jobstop = function(id)
    fake.stopped[#fake.stopped + 1] = id
    return 1
  end

  local ok, err = xpcall(function()
    fn(fake)
  end, debug.traceback)

  local loaded = package.loaded["lazydb"]
  if loaded and loaded._reset then
    pcall(loaded._reset)
  end
  vim.fn.jobstart = original_jobstart
  vim.fn.jobstop = original_jobstop

  if not ok then
    error(err, 0)
  end
end

local function contains(list, value)
  for _, item in ipairs(list) do
    if item == value then
      return true
    end
  end
  return false
end

test("exports the Lua API and user commands", function()
  local lazydb = fresh_plugin()

  for _, name in ipairs({ "setup", "open", "toggle", "hide", "stop", "restart", "status" }) do
    eq(type(lazydb[name]), "function", "missing Lua API: " .. name)
  end

  for _, name in ipairs({ "LazyDB", "LazyDBToggle", "LazyDBHide", "LazyDBStop", "LazyDBRestart" }) do
    eq(vim.fn.exists(":" .. name), 2, "missing command: " .. name)
  end
end)

test("merges configuration and builds a stable argv list", function()
  local lazydb = fresh_plugin()
  local config = require("lazydb.config")

  lazydb.setup({
    executable = "/tmp/lazydb fake",
    cwd = "/tmp/work tree",
    config = "/tmp/lazydb config.toml",
    profile = "local profile",
    read_only = true,
    mouse = "off",
    color = "always",
    window = { border = "single" },
  })

  local values = config.get()
  eq(values.window.border, "single")
  truthy(values.window.width, "nested window defaults were not preserved")
  eq(config.argv(), {
    "/tmp/lazydb fake",
    "--config",
    "/tmp/lazydb config.toml",
    "--profile",
    "local profile",
    "--read-only",
    "--mouse",
    "off",
    "--color",
    "always",
  })
  eq(config.cwd(), "/tmp/work tree")
end)

test("starts with argv and hides without stopping", function()
  with_fake_jobs(function(fake)
    local lazydb = fresh_plugin()
    local cwd = vim.fn.getcwd()
    lazydb.setup({ executable = "fake-lazydb", cwd = cwd, profile = "dev" })

    lazydb.open()
    eq(#fake.starts, 1)
    eq(fake.starts[1].argv, { "fake-lazydb", "--profile", "dev" })
    eq(fake.starts[1].opts.term, true)
    eq(fake.starts[1].opts.cwd, cwd)
    eq(type(fake.starts[1].argv), "table", "jobstart must receive an argv list")

    local running = lazydb.status()
    eq(running.state, "running")
    eq(running.visible, true)
    eq(running.job_id, fake.starts[1].id)

    local maps = vim.api.nvim_buf_get_keymap(running.buffer, "t")
    eq(maps, {}, "the plugin must not add terminal-mode mappings")

    local window = running.window
    vim.api.nvim_win_close(window, true)
    eq(lazydb.status().visible, false, "closing the float should only hide it")
    eq(fake.stopped, {}, "closing the float stopped the process")

    lazydb.open()
    eq(#fake.starts, 1, "reopening a hidden session started another process")
    eq(lazydb.status().visible, true)

    lazydb.toggle()
    eq(lazydb.status().visible, false)
    eq(fake.stopped, {})
    lazydb.toggle()
    eq(lazydb.status().visible, true)
    eq(#fake.starts, 1)

    truthy(lazydb.stop(), "stop should report an existing session")
    eq(lazydb.status().state, "stopped")
    eq(fake.stopped, { fake.starts[1].id })
    eq(lazydb.stop(), false, "stop should be idempotent")
  end)
end)

test("keeps one independent session per tab handle", function()
  with_fake_jobs(function(fake)
    local lazydb = fresh_plugin()
    lazydb.setup({ executable = "fake-lazydb" })

    local first_tab = vim.api.nvim_get_current_tabpage()
    lazydb.open()
    local first_job = lazydb.status().job_id

    vim.cmd("tabnew")
    local second_tab = vim.api.nvim_get_current_tabpage()
    lazydb.open()
    local second_job = lazydb.status().job_id

    eq(#fake.starts, 2)
    truthy(first_job ~= second_job, "tabs shared one job")

    vim.api.nvim_set_current_tabpage(first_tab)
    eq(lazydb.status().job_id, first_job)
    vim.api.nvim_set_current_tabpage(second_tab)
    eq(lazydb.status().job_id, second_job)

    vim.cmd("tabclose")
    wait_for(function()
      return contains(fake.stopped, second_job)
    end, "TabClosed did not stop its session")
    eq(lazydb.status().job_id, first_job, "closing another tab disturbed this session")
    lazydb.stop()
  end)
end)

test("ignores stale exits and preserves nonzero diagnostics", function()
  with_fake_jobs(function(fake)
    local lazydb = fresh_plugin()
    lazydb.setup({ executable = "fake-lazydb" })

    lazydb.open()
    local first = fake.starts[1]
    local first_buffer = lazydb.status().buffer
    lazydb.restart()

    local second = fake.starts[2]
    local restarted = lazydb.status()
    eq(restarted.job_id, second.id)
    truthy(restarted.buffer ~= first_buffer, "restart reused the old buffer")
    truthy(contains(fake.stopped, first.id), "restart did not stop the old job")

    first.opts.on_exit(first.id, 0, "exit")
    vim.wait(30)
    eq(lazydb.status().job_id, second.id, "a stale on_exit removed the restarted session")
    truthy(vim.api.nvim_buf_is_valid(restarted.buffer), "a stale on_exit wiped the new buffer")

    second.opts.on_exit(second.id, 7, "exit")
    wait_for(function()
      return lazydb.status().state == "exited"
    end, "nonzero exit was not recorded")
    local failed = lazydb.status()
    eq(failed.exit_code, 7)
    eq(failed.job_id, nil)
    truthy(vim.api.nvim_buf_is_valid(failed.buffer), "nonzero diagnostics were discarded")

    second.opts.on_exit(second.id, 7, "exit")
    vim.wait(30)
    eq(lazydb.status().state, "exited", "duplicate on_exit was not idempotent")
    lazydb.stop()

    lazydb.open()
    local normal = fake.starts[3]
    local normal_buffer = lazydb.status().buffer
    normal.opts.on_exit(normal.id, 0, "exit")
    wait_for(function()
      return lazydb.status().state == "stopped"
    end, "normal exit did not clean up the session")
    truthy(not vim.api.nvim_buf_is_valid(normal_buffer), "normal exit retained its buffer")
    normal.opts.on_exit(normal.id, 0, "exit")
  end)
end)

test("cleans up on buffer wipe and VimLeavePre", function()
  with_fake_jobs(function(fake)
    local lazydb = fresh_plugin()
    lazydb.setup({ executable = "fake-lazydb" })

    lazydb.open()
    local wiped_job = lazydb.status().job_id
    vim.api.nvim_buf_delete(lazydb.status().buffer, { force = true })
    wait_for(function()
      return lazydb.status().state == "stopped"
    end, "BufWipeout did not clean up the session")
    truthy(contains(fake.stopped, wiped_job), "BufWipeout did not stop the job")

    lazydb.open()
    local leaving_job = lazydb.status().job_id
    vim.api.nvim_exec_autocmds("VimLeavePre", {})
    eq(lazydb.status().state, "stopped")
    truthy(contains(fake.stopped, leaving_job), "VimLeavePre did not stop the job")
  end)
end)

test("health checks capabilities through the non-connecting CLI command", function()
  local lazydb = fresh_plugin()
  lazydb.setup({ executable = "fake-lazydb" })

  local original_executable = vim.fn.executable
  local original_system = vim.system
  local seen_argv

  vim.fn.executable = function(command)
    return command == "fake-lazydb" and 1 or 0
  end
  vim.system = function(argv, opts)
    seen_argv = vim.deepcopy(argv)
    eq(opts.text, true)
    return {
      wait = function()
        return {
          code = 0,
          stdout = '{"version":"0.1.0","cli_api":1,"features":["mouse","read-only","context-help","profile-manager","system-keyring"],"drivers":["postgres","mysql","sqlite"]}',
          stderr = "",
        }
      end,
    }
  end

  local ok, err = xpcall(function()
    require("lazydb.health").check()
  end, debug.traceback)
  vim.fn.executable = original_executable
  vim.system = original_system

  if not ok then
    error(err, 0)
  end
  eq(seen_argv, { "fake-lazydb", "capabilities", "--json" })
end)

function M.run()
  local failures = {}

  for _, item in ipairs(tests) do
    local ok, err = xpcall(item.fn, debug.traceback)
    if ok then
      print("ok - " .. item.name)
    else
      failures[#failures + 1] = item.name .. "\n" .. err
      print("not ok - " .. item.name)
    end
  end

  unload_plugin()

  if #failures > 0 then
    print(table.concat(failures, "\n\n"))
    vim.cmd("cquit 1")
    return
  end

  print(string.format("%d tests passed", #tests))
end

return M
