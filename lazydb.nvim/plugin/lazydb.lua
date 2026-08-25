if vim.g.loaded_lazydb_nvim then
  return
end
vim.g.loaded_lazydb_nvim = 1

require("lazydb")._register_commands()
