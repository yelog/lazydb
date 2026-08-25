local source = debug.getinfo(1, "S").source:sub(2)
local tests_dir = vim.fs.dirname(vim.fs.normalize(source))
local plugin_root = vim.fs.dirname(tests_dir)

vim.opt.runtimepath:prepend(plugin_root)
vim.opt.swapfile = false
vim.opt.shadafile = "NONE"

package.path = table.concat({
  tests_dir .. "/?.lua",
  plugin_root .. "/lua/?.lua",
  plugin_root .. "/lua/?/init.lua",
  package.path,
}, ";")
