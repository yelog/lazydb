# Changelog

## [0.1.0] - 2026-09-04

### Added

- Stable release line 0.1.0 consolidates the database workspace, coding-agent access, in-app update channels, release automation, and Windows installation support.

### Changed

- Refined database exploration, SQL editing, catalog operations, connection profiles, workspace persistence, transactions, result views, and terminal interaction across the 0.1.0 release line.

### Internal

- Release automation, CI compatibility, packaging, tests, and documentation were expanded and stabilized.

### Commits

- [`d6b321a`](https://github.com/yelog/lazydb/commit/d6b321a37be0ff52d1ed00e686eea07ec0f05801) feat: add Windows installation support
- [`bda9c42`](https://github.com/yelog/lazydb/commit/bda9c42e940b92473e2f88e9faa4b207f9ebfa09) fix(postgres): prioritize table object refreshes
- [`ddc4610`](https://github.com/yelog/lazydb/commit/ddc4610681f4c0055933c7b2b50326c3955d9207) fix(editor): ignore unsupported super key input
- [`59bee2f`](https://github.com/yelog/lazydb/commit/59bee2f7b5404d3905957017710a3d105212ed6e) merge: add macOS Command undo shortcuts
- [`8a59644`](https://github.com/yelog/lazydb/commit/8a5964414ff5a88389cff3175be0c50dbbc508d1) feat(input): use Command for undo on macOS
- [`bee5448`](https://github.com/yelog/lazydb/commit/bee5448e0c5044781b6d1fbf999f2f7b3c3d5ecb) style(ui): adjust SQL editor syntax colors
- [`4144589`](https://github.com/yelog/lazydb/commit/4144589ca444c8b6d700c1599fa31b05c4dca30c) merge: redesign catalog view and sequence forms
- [`c3130b1`](https://github.com/yelog/lazydb/commit/c3130b1581486844403f98fd4b9b3d9aee82c4ad) chore: satisfy catalog form clippy checks
- [`91b9352`](https://github.com/yelog/lazydb/commit/91b9352fe41be7e4526c0dfb904083a6dc6aad80) test(catalog): cover redesigned object forms
- [`0b24e54`](https://github.com/yelog/lazydb/commit/0b24e5405e76c5c5816aaebf5b76c659b17f9385) fix(catalog): focus invalid form fields
- [`2fbbecf`](https://github.com/yelog/lazydb/commit/2fbbecfb35e5cc49b6b0dce634f6c59040e2cd64) feat(catalog): wire form mouse and owner interactions
- [`af2d4c5`](https://github.com/yelog/lazydb/commit/af2d4c57adb4175f3b553282427f6c126b5521f5) feat(ui): redesign materialized view and sequence forms
- [`d8ad1a2`](https://github.com/yelog/lazydb/commit/d8ad1a2b0fa1fe32e4f5074847050e361c7ba3c8) fix(ui): keep focused view fields visible in compact forms
- [`b0e704f`](https://github.com/yelog/lazydb/commit/b0e704f739bceee3a9af08de51dc9b7adb7ec5a1) feat(ui): redesign catalog view form
- [`c4e6f3c`](https://github.com/yelog/lazydb/commit/c4e6f3c9c371d0d59a8644a4c9cce2ef7f6efef6) refactor(ui): share catalog form field rendering
- [`76eafd5`](https://github.com/yelog/lazydb/commit/76eafd5c24663ac0e1da6d0e73e6a1114163f015) feat(catalog): edit view options and sequence controls
- [`0d2e04c`](https://github.com/yelog/lazydb/commit/0d2e04cc97145230ce283e81b951ec7031137cc3) merge: retain semantic highlighting for incomplete SQL
- [`ac14de0`](https://github.com/yelog/lazydb/commit/ac14de04fd5b06b310345326ea0afe2e8ccf7b9e) fix(sql): retain semantic highlighting for incomplete input
- [`52baf2f`](https://github.com/yelog/lazydb/commit/52baf2f5792b76e89d80f2ac9a6d95a3a79aeec8) fix(catalog): separate materialized view storage controls
- [`0e77c25`](https://github.com/yelog/lazydb/commit/0e77c2505bd8e17045cb9bf2dcc0839fa5eb34b0) merge: add in-app update workflow
- [`188ac76`](https://github.com/yelog/lazydb/commit/188ac768598f9b3289a5afbfe858b2e53f1623a0) feat(update): add in-app update workflow
- [`82615e8`](https://github.com/yelog/lazydb/commit/82615e832e5362eb2ba6b44810defcf4b7e23e38) merge: add focused text undo redo
- [`0c558d7`](https://github.com/yelog/lazydb/commit/0c558d788abba010456daf3aebf840b1e582c2f1) refactor(catalog): type view and sequence form focus
- [`8929a3e`](https://github.com/yelog/lazydb/commit/8929a3e92daa4ba18732127c7e890f7ebfe57faa) feat(input): add focused undo and redo
- [`5ffff9c`](https://github.com/yelog/lazydb/commit/5ffff9cb6df0132bd98aa09a3aa8aeeca9345fa7) fix(catalog): refresh table group counts after mutation
- [`64283d2`](https://github.com/yelog/lazydb/commit/64283d2b32d87f0c260f7b5803a1be5c19e14dd6) merge: add semantic SQL highlighting
- [`45a2b08`](https://github.com/yelog/lazydb/commit/45a2b08b584b4157f56b6c8c43bfe7714e2dc8ee) feat(editor): add semantic SQL highlighting
- [`d7ff8a1`](https://github.com/yelog/lazydb/commit/d7ff8a1d96ef94115fb395dc9ca9b84cd2f20fa8) merge: move table column details into modal
- [`32695e0`](https://github.com/yelog/lazydb/commit/32695e08b72a228c3a35bcfef41cde5497760d25) feat(catalog): move table column details into modal
- [`32ab225`](https://github.com/yelog/lazydb/commit/32ab2251abd78bd7a1ae5c365a24003e9130f8c5) fix(editor): restore normal mode gg motion
- [`d360f6c`](https://github.com/yelog/lazydb/commit/d360f6c0d61ede70f9077f9c18a4daf655f3aa67) merge: unify SQL editor undo redo transactions
- [`ec78d86`](https://github.com/yelog/lazydb/commit/ec78d866443dc3dfc3925d3662669992181da02f) fix(editor): unify undo redo transactions
- [`c6ca8ca`](https://github.com/yelog/lazydb/commit/c6ca8ca24fb278a07fe67898e1caf807ddbdd182) fix(catalog-editor): support table field paste
- [`f58c06d`](https://github.com/yelog/lazydb/commit/f58c06d885acaa76f0f72ff70e8d926f81872eee) test(sqlserver): avoid read committed lock wait
- [`1301748`](https://github.com/yelog/lazydb/commit/13017484ceb2e401432f118054354ee862ac2778) test(sqlserver): trace transaction fixture progress
- [`c45e905`](https://github.com/yelog/lazydb/commit/c45e905e2c62e0c9f69204d7e5375edc4578805e) test(sqlserver): avoid force close integration hang
- [`68472d1`](https://github.com/yelog/lazydb/commit/68472d12d324665f3a5319c4b926f129bf720c45) test(sqlserver): close clean transaction sessions
- [`007738e`](https://github.com/yelog/lazydb/commit/007738e22d6161f84723bcad5f9d058344211c17) test(sqlserver): avoid reusing timed out connection
- [`6346c01`](https://github.com/yelog/lazydb/commit/6346c015f09eda6ecc869cdd23eecddbd316374e) test(runtime): stabilize asynchronous connection assertions
- [`82c139d`](https://github.com/yelog/lazydb/commit/82c139df28cba834618be01c0eecdf95675416a0) Merge task/new-table-focus-execution
- [`1930a4c`](https://github.com/yelog/lazydb/commit/1930a4c8c6c36bc9b1a8c8598de7e4ba58c1bbb4) test(sqlserver): use updated rowversion for delete
- [`f606060`](https://github.com/yelog/lazydb/commit/f6060602681b219720f990ce86529eec5a442851) fix(catalog-editor): correct table focus and mutation execution
- [`b463432`](https://github.com/yelog/lazydb/commit/b46343262b5c0a162fbb33ee876eb8362d92fb31) fix(sqlserver): bound savepoint names
- [`7c9c259`](https://github.com/yelog/lazydb/commit/7c9c259cdd7392bd3f6e29b467d922fb78f7df20) Merge query loading state redesign
- [`8529cbe`](https://github.com/yelog/lazydb/commit/8529cbee04044a1530ab0a0180af918e405f73fc) perf(ui): simplify query loading feedback
- [`1ab63ca`](https://github.com/yelog/lazydb/commit/1ab63ca46b93d9f9f9d8093d6afdf80275a5ac05) fix(sqlserver): shorten mutation savepoints
- [`28dad1a`](https://github.com/yelog/lazydb/commit/28dad1af1a7376cf5a4569ed3da37c8606ebd9ce) ci: isolate database integration test timeouts
- [`d5926f8`](https://github.com/yelog/lazydb/commit/d5926f8455d028df44a4f9c3ff9e963dd62fd9e6) fix(update): unify config and install state paths
- [`2b0b2cb`](https://github.com/yelog/lazydb/commit/2b0b2cb8bcbaf14fc493f210cb7b57518c5dd39c) fix(sqlserver): decode catalog flags as bits
- [`3f0c629`](https://github.com/yelog/lazydb/commit/3f0c629509b07b552588fd33ec6facd68474b7f8) fix(sqlserver): avoid stale relation catalog validation
- [`1fd7c4e`](https://github.com/yelog/lazydb/commit/1fd7c4e5ddc400c62ceb6a94857a2e4d8c52145e) fix(sqlserver): use stable relation identity
- [`8dd54d4`](https://github.com/yelog/lazydb/commit/8dd54d40347aae4626981c24769d33ac3fc63b32) fix(sqlserver): restore relation id validation
- [`51a424b`](https://github.com/yelog/lazydb/commit/51a424bb5a2ae64efb2407c4f854fc444f5a1d55) fix(sqlserver): preserve object ids for catalog queries
- [`b839018`](https://github.com/yelog/lazydb/commit/b8390185540e94e516ff72564bc9051a149f1d12) fix(sqlserver): retain relation target validation
- [`f6e62eb`](https://github.com/yelog/lazydb/commit/f6e62eb7c29b4cb2879f6432018fac9b80caeaee) fix(sqlserver): validate relations by name
- [`963cbe3`](https://github.com/yelog/lazydb/commit/963cbe39692db591e87bac688b09fe1bab006b07) fix(sqlserver): isolate create table batches
- [`7f9ccce`](https://github.com/yelog/lazydb/commit/7f9cccea0a400c942102cfbeb554ac434e99d1b9) fix(sqlserver): keep all create batches standalone
- [`22e4fd8`](https://github.com/yelog/lazydb/commit/22e4fd8cc11a7035d8c4a87bcb6bcf77e30692d4) fix(sqlserver): execute standalone DDL batches
- [`e6949a6`](https://github.com/yelog/lazydb/commit/e6949a68146a79a208f5ffaf866c2780496583e2) fix(sqlserver): align integration tests with supported syntax
- [`72cb2d7`](https://github.com/yelog/lazydb/commit/72cb2d75dcbbd6c0868f52bcaf83ab147d75d4b8) fix(ci): pin SQL Server integration image
- [`cc3f0c9`](https://github.com/yelog/lazydb/commit/cc3f0c9c3f6d8972b45ab423089ebc24f5b3f007) fix(ci): derive connection access default
- [`5ff230d`](https://github.com/yelog/lazydb/commit/5ff230d9547d47b137f64d89111082480a2b6311) fix(catalog): keep focus movable on the schema owner row
- [`e2e227a`](https://github.com/yelog/lazydb/commit/e2e227ac00d7c715087bb53707697af4795af995) merge: add PostgreSQL schema owner picker
- [`afa24f4`](https://github.com/yelog/lazydb/commit/afa24f4784c4db7a153fc553cc5bf81a3dc81b78) feat(catalog): add PostgreSQL schema owner picker
- [`f971a97`](https://github.com/yelog/lazydb/commit/f971a97622bc7a6fa887dace265a514bacdbaecf) Merge branch 'task/new-table-panel'
- [`37c1e1c`](https://github.com/yelog/lazydb/commit/37c1e1cc5f6c27b48df134bc2444d40fc84ecd91) feat(catalog): make new table panel editable
- [`079d284`](https://github.com/yelog/lazydb/commit/079d28472a0c8ad7ff1f0139f5dd04a42071f8ce) merge: differentiate shortcut keys and actions
- [`f75d02e`](https://github.com/yelog/lazydb/commit/f75d02e8035367c3f5cc34f901c38b62d7d9a705) fix(ui): differentiate shortcut keys and actions
- [`77e5649`](https://github.com/yelog/lazydb/commit/77e5649864ee37484e12001291c88c3dd9aae4ee) feat(config): configure default connection access
- [`753296d`](https://github.com/yelog/lazydb/commit/753296de05e7b7010ceb73aa28fce3e71c886bf8) fix(catalog): support schema field editing
- [`9b56fdc`](https://github.com/yelog/lazydb/commit/9b56fdc50124db06abb7e4c75071e18ad76bbbb2) fix(input): normalize shift-tab pane navigation
- [`c39a8cf`](https://github.com/yelog/lazydb/commit/c39a8cfaa3587ebb7c97378a31f3f46e2e7c1d98) merge: redesign pending transaction panel
- [`a47f8c5`](https://github.com/yelog/lazydb/commit/a47f8c59b69fa5bbdc4136d3469247336b3391fa) test(ui): verify compact transaction panel
- [`6945857`](https://github.com/yelog/lazydb/commit/69458576034d7af10ca854040fccce85b7487355) fix(ui): clarify pending transaction states
- [`7241c4c`](https://github.com/yelog/lazydb/commit/7241c4c7d6931943b595063725b111835663c5ca) docs(plans): add upcoming implementation plans
- [`7c63911`](https://github.com/yelog/lazydb/commit/7c63911b04136e94b90147b80343346a05505167) test(ui): cover transaction panel states
- [`7d0502d`](https://github.com/yelog/lazydb/commit/7d0502da303c32d5b048338e9adce734bd3c17a2) merge: streamline explorer catalog creation
- [`9815d75`](https://github.com/yelog/lazydb/commit/9815d75ba48309c738b1b554d38e72a9a9893375) feat(catalog): streamline explorer create experience
- [`90160b3`](https://github.com/yelog/lazydb/commit/90160b3774c86af5c70dbe05753e3bb73c0f780d) docs(config): group keybindings by panel
- [`c80eb44`](https://github.com/yelog/lazydb/commit/c80eb44ab0fa8e65119f075377e4f5bf8c81d7a2) refactor(ui): redesign pending transaction panel
- [`c9ba4cb`](https://github.com/yelog/lazydb/commit/c9ba4cb4f14638fd28b082f10c8aca9060179158) feat(config): migrate result grid bindings
- [`5a8e190`](https://github.com/yelog/lazydb/commit/5a8e1906ade6b3420b45aa93ae5feb7b69d23c6f) feat(config): migrate contextual navigation bindings
- [`40b7dee`](https://github.com/yelog/lazydb/commit/40b7dee8034e6e6c3cae6f7691e157c1603339d7) merge: sync main into completion type alignment
- [`5fd44e5`](https://github.com/yelog/lazydb/commit/5fd44e5d9fe21489020c6833da2170da5eb13bdb) merge: compact workspace status chrome
- [`06e1ef7`](https://github.com/yelog/lazydb/commit/06e1ef7cb8d830e02110a60b569196c8a5b783f9) feat(ui): compact workspace status chrome
- [`7f63829`](https://github.com/yelog/lazydb/commit/7f63829afaa02772864a4e1534b1d39330ef7691) test(sql): update completion connection fixture
- [`ca1113e`](https://github.com/yelog/lazydb/commit/ca1113ed9899f87f5b5977e84bb0d5385872e510) merge: add contextual ddl completion
- [`aa056a0`](https://github.com/yelog/lazydb/commit/aa056a0e3fc46e5db1a3499f12430aef4b740690) feat(sql): add contextual ddl completion
- [`2b2b688`](https://github.com/yelog/lazydb/commit/2b2b688a2cc627753339021f111286bd0dc53616) merge: right-align and shorten completion types
- [`34248cd`](https://github.com/yelog/lazydb/commit/34248cd1ed262bcb9ed99800949137dc57885cf7) feat(ui): right-align and shorten completion types
- [`0c2ca6d`](https://github.com/yelog/lazydb/commit/0c2ca6d0f0eb1fad6bf3f3491bc002d64406c352) merge: unify catalog create capabilities
- [`fd5dc86`](https://github.com/yelog/lazydb/commit/fd5dc864f758097b4020843f55fa1b52d972f4c0) fix(catalog): unify create object capabilities
- [`3a8485b`](https://github.com/yelog/lazydb/commit/3a8485b2cf7f7a66f84aba6c227dc391e13806c6) feat(config): migrate contextual shortcuts
- [`0796fda`](https://github.com/yelog/lazydb/commit/0796fda3cc52c7edbc71c5e3c4d116b3dab13e71) merge: add ctrl-w focus cycling
- [`4cde68c`](https://github.com/yelog/lazydb/commit/4cde68cab27d04d5ea4c9e8424daf54ba88beade) feat(keymap): add ctrl-w focus cycling
- [`a00f255`](https://github.com/yelog/lazydb/commit/a00f255a8c8a8f53e7a78152b5d95b4771e6c4f8) merge: complete insert target columns
- [`0e7286b`](https://github.com/yelog/lazydb/commit/0e7286b04e25c82982fbc80813ea174269ee7e84) fix(sql): complete insert target columns
- [`52a080c`](https://github.com/yelog/lazydb/commit/52a080cbbd3966844d21af75f07441b36379d145) feat(config): support configurable key sequences
- [`86e0fc8`](https://github.com/yelog/lazydb/commit/86e0fc8c3c2e299cd817b6320d89d24e63244200) feat(config): migrate more keybindings
- [`49ce1da`](https://github.com/yelog/lazydb/commit/49ce1da1ec9761c1539634c89f4f04d28a2bc45f) merge: insert into completion
- [`5611508`](https://github.com/yelog/lazydb/commit/561150868bac8cfe2bed6d88ff2a43007fe044a7) fix(sql): suggest into after insert
- [`1852e55`](https://github.com/yelog/lazydb/commit/1852e552dbfa5d57c7328d971aa4143bbfeeedd0) fix(input): preserve insert-mode characters
- [`120917a`](https://github.com/yelog/lazydb/commit/120917a54c158a255b3ab5e961a573fef7a91ca2) fix(input): preserve contextual key precedence
- [`e62c019`](https://github.com/yelog/lazydb/commit/e62c019841980778df1a63a69dc728d141bbbe15) merge: add explorer connection action menu
- [`a4f766e`](https://github.com/yelog/lazydb/commit/a4f766eeec68c11c3aa9ddc48a337eba5162c927) feat(explorer): add connection action menu
- [`19bc652`](https://github.com/yelog/lazydb/commit/19bc65285172d3b18c4042ec2942ebbc2eb58104) chore: commit remaining workspace changes
- [`7cebf9a`](https://github.com/yelog/lazydb/commit/7cebf9a5fb1b54810fe14aea0199f181c837b57c) feat(config): add configurable global keybindings
- [`64cc5cf`](https://github.com/yelog/lazydb/commit/64cc5cff27da3803cb19d8f2156134612dffbc4e) test(ui): cover dashboard pane maximize
- [`0e42f19`](https://github.com/yelog/lazydb/commit/0e42f190c9154468e13cb7f58c93482486384795) feat(config): add embedded application defaults
- [`f81f6f9`](https://github.com/yelog/lazydb/commit/f81f6f9bb86ab9e04fb45adcf6b57b01df02a227) fix(input): handle transaction toggle sequence
- [`d6518aa`](https://github.com/yelog/lazydb/commit/d6518aa27f7c742afd94c408d66d273e70827370) fix(input): restore transaction toggle shortcut
- [`1126e68`](https://github.com/yelog/lazydb/commit/1126e684e07ccbc91613751f1277aa19c2b01bcf) fix(explorer): select grouped connection nodes on startup
- [`228bbf8`](https://github.com/yelog/lazydb/commit/228bbf8311cc071be506500dc31512713307b003) feat(ui): add focused pane maximize
- [`cfedad5`](https://github.com/yelog/lazydb/commit/cfedad59a6bc57e6696eb29c1f3c05fb9ba0fbb0) chore: add implementation plans and format db modules
- [`9938a7a`](https://github.com/yelog/lazydb/commit/9938a7a0b11949f8d3ed725ad50650fd86fa8e4d) fix(explorer): expose grouped connection goto
- [`39ebd92`](https://github.com/yelog/lazydb/commit/39ebd92d5cdb1adbcba569851a3e89d0a0a475b1) test(ui): cover focused pane maximize rendering
- [`4a0faa2`](https://github.com/yelog/lazydb/commit/4a0faa2cd78a6fead11b498395decb9ecea5a682) feat(input): unify single-line editing controls
- [`3265e16`](https://github.com/yelog/lazydb/commit/3265e162cf8aa27a54a8895496025c257e3f1aff) feat(help): document focused pane maximize
- [`af0786b`](https://github.com/yelog/lazydb/commit/af0786b3f099595ea0fe0205db8920ff6d4a8ed2) test(explorer): cover group expansion actions
- [`28697e1`](https://github.com/yelog/lazydb/commit/28697e1aafd6cc658a3c1d3c3b320aade78ffa72) feat(ui): render focused pane in maximize mode
- [`048c120`](https://github.com/yelog/lazydb/commit/048c12085f17cf43da829618c83a5c06f57cf4a4) feat(editor): support pane maximize window command
- [`e9f6893`](https://github.com/yelog/lazydb/commit/e9f68930cb2dececaa512e8ce7c32809c846c4f8) feat(keymap): map ctrl-w f to pane maximize
- [`ffc43b2`](https://github.com/yelog/lazydb/commit/ffc43b25f502c13475a71263b25eb5c2dcca4be4) feat(ui): add pane maximize state
- [`892b171`](https://github.com/yelog/lazydb/commit/892b1711f721820ab733f8dcd570a31eab57975a) fix(explorer): allow connection groups to expand
- [`4ea2864`](https://github.com/yelog/lazydb/commit/4ea2864eef6fd77f8116104a6ebcb653bc14f4a0) merge: integrate console manager
- [`5165f75`](https://github.com/yelog/lazydb/commit/5165f751dd3f571cdbc47d0fe8965638001ee4ac) feat(console): add unified console manager
- [`2fcdbd4`](https://github.com/yelog/lazydb/commit/2fcdbd46d0ffdbadf7351a9db580292e6debc7f5) fix(help): document Explorer group shortcuts
- [`1e163a3`](https://github.com/yelog/lazydb/commit/1e163a3e38883ca9be42a2fdde8ef101eb58f7dd) test(keymap): align stale completion shortcut expectations
- [`aae6e96`](https://github.com/yelog/lazydb/commit/aae6e96d8ffa4843417acfd9864fd66faa72f120) merge: integrate SQL Server support
- [`55e91ba`](https://github.com/yelog/lazydb/commit/55e91ba351bcc99b9a478277987200a29029af3c) fix(tabs): focus editor when switching consoles
- [`0e0eaa2`](https://github.com/yelog/lazydb/commit/0e0eaa21c1353dea54401b6c76337f8a8b897472) fix(explorer): show newly created empty groups
- [`50a4cd7`](https://github.com/yelog/lazydb/commit/50a4cd7d8ff0354065501e1053a5accd0c1bdf50) feat(sqlserver): add SQL Server support
- [`6331116`](https://github.com/yelog/lazydb/commit/63311161c375f12b6a8f1f71e5a8b91cd09886fc) feat(input): update tab keyboard shortcuts
- [`29e2e60`](https://github.com/yelog/lazydb/commit/29e2e60af363e27b78851e54b464adc3575bc4ce) fix(explorer): route group editor text keys
- [`a83e026`](https://github.com/yelog/lazydb/commit/a83e02613f51205661cedf71bd3beccbb0490ec3) fix(explorer): open group creation from connections
- [`f3c6204`](https://github.com/yelog/lazydb/commit/f3c620427a1053dca9b3876f2e77c5491fbcfb4c) merge: integrate Explorer connection groups
- [`2b936f7`](https://github.com/yelog/lazydb/commit/2b936f751cd84834676abfabefb899b286afeffa) feat(explorer): add connection groups and ordering
- [`3f41237`](https://github.com/yelog/lazydb/commit/3f4123766cd19fd0f908daff3a4d2cc1aa09b4cd) merge: integrate workspace tab overflow
- [`f3fe1c3`](https://github.com/yelog/lazydb/commit/f3fe1c322bbf29e38bd9915d22f561e106591ab8) feat(ui): keep active workspace tabs visible
- [`bf243df`](https://github.com/yelog/lazydb/commit/bf243dff2433f56b42445c286ce21ec7a222c94c) fix(ci): satisfy newer clippy checks
- [`964bc82`](https://github.com/yelog/lazydb/commit/964bc82b173dfdabfa5c01e4f7423cbd93256f6b) merge: integrate explorer catalog object editor
- [`fd90bf3`](https://github.com/yelog/lazydb/commit/fd90bf3725b22edfc6c20d82c8b90d2c921175c6) feat(explorer): add catalog object editor
- [`a65396e`](https://github.com/yelog/lazydb/commit/a65396ef3b19b861a00d97bf2c36af68d4b32964) fix(dashboard): place refresh status in tab header
- [`722f28b`](https://github.com/yelog/lazydb/commit/722f28b7e94ebe4df080982832c884af910efeea) merge: integrate disconnected workspace empty state
- [`80c0382`](https://github.com/yelog/lazydb/commit/80c03829056fbd5360b9386bfb28018b0feb3a2f) feat(ui): guide disconnected workspace users
- [`963b734`](https://github.com/yelog/lazydb/commit/963b73436637b42da5e0a5fd97f832139d9e3e00) feat(dashboard): add overview and process list tabs
- [`7fada35`](https://github.com/yelog/lazydb/commit/7fada354fd30382486bb59f6a5ac71c163af09a4) feat(dashboard): complete process list grid interactions
- [`3e748f1`](https://github.com/yelog/lazydb/commit/3e748f1a30d2bceaa4bdd53b5896e954663d7d48) fix(dashboard): use shared settings path
- [`25c548e`](https://github.com/yelog/lazydb/commit/25c548e5781109528514c7d74290de40f964e193) feat(dashboard): configure monitoring refresh interval
- [`245202d`](https://github.com/yelog/lazydb/commit/245202d9cec56f7cd273fc2b84bc8487ec3bfdfb) fix(paths): use configurable Unix config directory
- [`bd636c7`](https://github.com/yelog/lazydb/commit/bd636c7156f966994eaf0e53a084f75364df1bdf) feat(dashboard): improve focus, activity charts, and shortcuts
- [`eb9bf37`](https://github.com/yelog/lazydb/commit/eb9bf370dcd6e7a4aa394f5833655719c7675ead) fix(dashboard): restore PostgreSQL metrics and overview history
- [`b22d2f6`](https://github.com/yelog/lazydb/commit/b22d2f6965e69b0d66e23e47e58d24092810a0c3) Merge branch 'task/database-dashboard'
- [`6d64b94`](https://github.com/yelog/lazydb/commit/6d64b94df00275fbe9912362eaec8903ee549bda) merge: integrate SQL completion popup polish
- [`c608efc`](https://github.com/yelog/lazydb/commit/c608efcd6577ce9a21c1246a70101b2e2a17a45c) Merge branch 'main' into task/database-dashboard
- [`f73c73a`](https://github.com/yelog/lazydb/commit/f73c73add2ca2b39ceb5882b00483a9b6d2ca2d0) Merge branch 'main' into task/sql-completion-popup-polish
- [`6aad101`](https://github.com/yelog/lazydb/commit/6aad1015bcd2f2b6f00a1ac10df5527160f28cc6) feat(dashboard): add database monitoring workspace
- [`7bfa6c1`](https://github.com/yelog/lazydb/commit/7bfa6c1b7d862afaca40e271820c7638d587408a) feat(ui): polish SQL completion popup
- [`0b60af6`](https://github.com/yelog/lazydb/commit/0b60af66f33437bf10bedd6904bfbcb319280f8a) fix(paths): unify workspace storage and migrate legacy files
- [`9f74ce6`](https://github.com/yelog/lazydb/commit/9f74ce62f77568da0e7fd11b7937a7ea5c14b02d) fix(profiles): merge identical button style branches
- [`247de15`](https://github.com/yelog/lazydb/commit/247de15f90bb556010fd7156313d2efba7e97e7a) fix(paths): use predictable platform config directories
- [`52b39b7`](https://github.com/yelog/lazydb/commit/52b39b78117185d3e6f14b5e48a1d0b7c654de6a) fix(sql): prefer FROM after completed projections
- [`69caffe`](https://github.com/yelog/lazydb/commit/69caffe5509ada7d81c3cda0f4480cf1b16046c2) fix(keymap): use e to edit relation cells
- [`52c428c`](https://github.com/yelog/lazydb/commit/52c428c902a0c8b749835f9bd9ab4502f275d99e) feat(profiles): redesign connection form UI
- [`794b328`](https://github.com/yelog/lazydb/commit/794b3286202dc654e5b8035d57d598c1ff519ed3) merge: integrate MCP write policy diagnostics
- [`38fea0d`](https://github.com/yelog/lazydb/commit/38fea0db2db2a8dccd99de251185771ba66efc1a) docs: simplify installation and quick start
- [`27473c9`](https://github.com/yelog/lazydb/commit/27473c91d1fee8cc501a7e7baa4ec551457e8c04) docs(mcp): explain write policy troubleshooting
- [`13e53bc`](https://github.com/yelog/lazydb/commit/13e53bc0ee7861454fae310e2de89f8e80cd0444) docs(mcp): clarify write authorization layers
- [`137dbe9`](https://github.com/yelog/lazydb/commit/137dbe9bd9a5e48b192bb05b18ecb5bf0b66b59b) feat(mcp): report effective write capability
- [`5bf76b1`](https://github.com/yelog/lazydb/commit/5bf76b1b230b7cf13ba0fde4b6a7f8fa82cb30a5) feat(profiles): improve connection form controls
- [`24ac8f3`](https://github.com/yelog/lazydb/commit/24ac8f3644b85cb5a9d2c7976f1f076c43102ad4) feat(agent): model effective write capability
- [`51f202f`](https://github.com/yelog/lazydb/commit/51f202f1411fe99d67f76f621c9ee41c8bc91e2a) fix(agent): explain rejected write operations
- [`10bc79b`](https://github.com/yelog/lazydb/commit/10bc79b42bbff93d23bd4506af14f3b20ffd258a) refactor(agent): distinguish write policy denials
- [`8ce1036`](https://github.com/yelog/lazydb/commit/8ce10366a65ca8e191d5265d07a089c63d266ee7) fix(explorer): make others group actionable
- [`f738de2`](https://github.com/yelog/lazydb/commit/f738de29fa71a955efc32f312daa4c3417987a0d) docs: document configuration options and files
- [`cbcaba9`](https://github.com/yelog/lazydb/commit/cbcaba9d4413f9ce138432fb1d22c13181731c60) fix(release): deploy Pages after release workflow
- [`ad10734`](https://github.com/yelog/lazydb/commit/ad107340f199df28714273800d0a1e6a8ee406f9) chore(release): prepare v0.1.0-beta.2
- [`d34fd20`](https://github.com/yelog/lazydb/commit/d34fd20ae31feb2fb27357022d9d34a33bae38f2) test(postgres): avoid assuming search returns relation children
- [`93b6970`](https://github.com/yelog/lazydb/commit/93b6970a54f7bed17ff43dd6efdb576267fd6d21) test(postgres): assert column metadata separately
- [`4de41a0`](https://github.com/yelog/lazydb/commit/4de41a0ea3c41299ed28a885c34bd606908b53e8) test(postgres): use valid bounded column search
- [`e220f26`](https://github.com/yelog/lazydb/commit/e220f265ece56613e543aaf125b0ff15a40e54ac) test(postgres): avoid truncating column search results
- [`aa755f1`](https://github.com/yelog/lazydb/commit/aa755f1d1abdcb906e2740e98670393591152488) test(postgres): allow room for relation search children
- [`4e284ec`](https://github.com/yelog/lazydb/commit/4e284ec6333bd67a0c4afb8d99c713ce2d1bd6a0) test(postgres): search table columns through relation match
- [`1b485e9`](https://github.com/yelog/lazydb/commit/1b485e9c5c8d308b617208a448b0c2a537c637e3) test(postgres): search columns by scoped name
- [`2f957d9`](https://github.com/yelog/lazydb/commit/2f957d9ea5f3312feeb43a76c43e722f65f1bcb0) test(postgres): match columns by relation suffix
- [`02b3411`](https://github.com/yelog/lazydb/commit/02b34114230dcc59ced0c30f229b74bd59743924) test(postgres): search columns by full catalog path
- [`8962c3e`](https://github.com/yelog/lazydb/commit/8962c3e7891062fe90096dad496bbe124d47f64b) ci: serialize database integration tests
- [`fa11787`](https://github.com/yelog/lazydb/commit/fa117871acb55e268c0f37386a57654409d4060e) fix(ci): resolve clippy and mysql test failures
- [`73e5417`](https://github.com/yelog/lazydb/commit/73e54179947ba759bb9143ae4f2f2b2f68af029b) fix(explorer): ignore separators in search
- [`d8100e6`](https://github.com/yelog/lazydb/commit/d8100e6a29f59f81a17c063658f24097dd7432ca) fix(keymap): enable leader shortcuts outside explorer
- [`5d8d557`](https://github.com/yelog/lazydb/commit/5d8d55785f95dadd74a75cf88423b0343cc8669a) fix(ui): distinguish null values in data views
- [`d34a03a`](https://github.com/yelog/lazydb/commit/d34a03a9715fedbad2d89fd8a4967a76f030b848) fix(sql): focus output when execution fails
- [`385638c`](https://github.com/yelog/lazydb/commit/385638cbf1521436023cc583f88882bd81b53e57) fix(keymap): preserve relation shortcuts
- [`0db886c`](https://github.com/yelog/lazydb/commit/0db886ce4cc64e5de70ed0fa62293a53305c71fb) merge: add notification center
- [`0bb2a1a`](https://github.com/yelog/lazydb/commit/0bb2a1a8b98e2b9f3fc4f14e77cd277e280e1b63) docs: add notification center design
- [`a3d027b`](https://github.com/yelog/lazydb/commit/a3d027b2ce392c970a67711d22569ca4c5909ff4) feat(ui): add notification center
- [`bf73ab8`](https://github.com/yelog/lazydb/commit/bf73ab84631fb1385f712e820741d5d4dce374cb) test(ui): focus transaction help assertion
- [`8b7fc54`](https://github.com/yelog/lazydb/commit/8b7fc548c76a7f07a3260aac6ec157981eb731d6) feat(ui): add persistent shortcut popup
- [`b289109`](https://github.com/yelog/lazydb/commit/b289109815cd0d9470f25e2333673a44172a1811) feat(ui): improve workspace tab controls
- [`cb69915`](https://github.com/yelog/lazydb/commit/cb69915ed1e35e2559249a193d3cf73bc835e206) fix(ui): show relation commit success
- [`2e35ebd`](https://github.com/yelog/lazydb/commit/2e35ebd1f5397252fb38c702612809164b6de138) fix(postgres): stabilize relation mutations
- [`163f754`](https://github.com/yelog/lazydb/commit/163f7547fd8ca740b71d1a30cc4b7996663f040d) fix(ui): highlight only edited relation cells
- [`41ab957`](https://github.com/yelog/lazydb/commit/41ab9573c49e86be151af58fa942db88f8163f9e) fix(pages): make installers standalone
- [`9adb3ef`](https://github.com/yelog/lazydb/commit/9adb3ef4b50f80b6b0a466aaf2c77346cf7406ca) docs: correct keyboard shortcut reference
- [`55ba036`](https://github.com/yelog/lazydb/commit/55ba036f013e5df05f03e58083cec42618c79af3) merge: add contextual keyboard hints
- [`575d98e`](https://github.com/yelog/lazydb/commit/575d98ef97eb9f2a26e055401e72c2d7f0a66a02) feat(ui): add contextual keyboard hints
- [`f0fccd3`](https://github.com/yelog/lazydb/commit/f0fccd38b41957126c7f9d9fc1414a2a4acc9e96) fix(sqlite): avoid unsupported returning syntax
- [`f465259`](https://github.com/yelog/lazydb/commit/f46525946c7b98103f73bec50267fbe1db7861fc) merge: add safe Explorer catalog drops
- [`41b3330`](https://github.com/yelog/lazydb/commit/41b3330ad626f038969de2fcb5e123a0b80c2212) feat(explorer): safely drop catalog objects
- [`caf95a6`](https://github.com/yelog/lazydb/commit/caf95a6d30b3ea9274cb0e120363c95bc76904cc) fix: stabilize relation table editing
- [`afb78a4`](https://github.com/yelog/lazydb/commit/afb78a45954f2506efb63d0a1982850f8c8c8ffc) fix: preserve pagination key bindings after merge
- [`d047495`](https://github.com/yelog/lazydb/commit/d0474952d2428b24b81d962a2750b63f2c66bdc7) merge: add paginated database results
- [`a60e7c7`](https://github.com/yelog/lazydb/commit/a60e7c7948bf08aeaad4debeece0b857413b24d4) feat: add paginated database results
- [`86d06cc`](https://github.com/yelog/lazydb/commit/86d06cc3c774155bc1d7f4bffc6ccb6d42fc7899) docs: add implementation plans
- [`18afd88`](https://github.com/yelog/lazydb/commit/18afd88460c8a3a559e135a737767eb52e29b60e) feat(ui): navigate completion candidates with arrows
- [`be519ef`](https://github.com/yelog/lazydb/commit/be519efa8edb42e375ee13163ef7065bf85da4be) merge: add context-aware SQL completion
- [`1f797e4`](https://github.com/yelog/lazydb/commit/1f797e47014518c6cab508c0c7a41d933b1b779a) feat(sql): make completion context-aware
- [`1d13745`](https://github.com/yelog/lazydb/commit/1d13745ee9cc96937aa2d67f9559faa231ec6643) fix(ui): distinguish data grid header from selection
- [`556809c`](https://github.com/yelog/lazydb/commit/556809c106de607e83d1693005274282898079c1) feat(ui): alias zero to first data grid column
- [`c135f54`](https://github.com/yelog/lazydb/commit/c135f54636a7383f329cf8cff9380577fdf28586) feat(ui): add data grid column jump shortcuts
- [`9489da3`](https://github.com/yelog/lazydb/commit/9489da361153f40b66a1aafc0b8daa63cc009089) fix(ui): lower result filter layout breakpoint
- [`40a278b`](https://github.com/yelog/lazydb/commit/40a278bd984d4cee833e2ebe971619102b06354c) fix(ui): place result filters inside panel
- [`76dc6fb`](https://github.com/yelog/lazydb/commit/76dc6fb3a29d03b09027ce454fe7cda4475567ae) test(ui): restore result completion coverage
- [`e984996`](https://github.com/yelog/lazydb/commit/e984996218dc2869916438d8eb6579c052fd7dec) merge: clarify SQL result filter lifecycle
- [`fb54bb6`](https://github.com/yelog/lazydb/commit/fb54bb654830576330563618b6c192246914dfce) fix(sql): clarify result filter lifecycle
- [`5d360e6`](https://github.com/yelog/lazydb/commit/5d360e66481a3a8805718f4041fca420a0dcf93a) merge: refine relation data hierarchy
- [`54b199e`](https://github.com/yelog/lazydb/commit/54b199e4db91732bf35b9f08b569cc68014a09df) feat(ui): refine relation data hierarchy
- [`3f41f6d`](https://github.com/yelog/lazydb/commit/3f41f6d2edbce4cbb7da4c44072edb745e830305) fix(ui): stabilize explorer expansion rendering
- [`a7fcfd7`](https://github.com/yelog/lazydb/commit/a7fcfd7eb55b520432762e9a192cc8cbd215c2b9) docs: add relation data visual hierarchy plans
- [`18596be`](https://github.com/yelog/lazydb/commit/18596be305894692917cd90ed2aa5ffb0d382dca) merge: add native install and update channels
- [`5d1a374`](https://github.com/yelog/lazydb/commit/5d1a37488dabdde5c3343a554e3a1cf9bdfcd526) feat(release): add native install and update channels
- [`dee9eb5`](https://github.com/yelog/lazydb/commit/dee9eb5e29d4d5326a4d8173dee17334f6752e9e) Merge branch 'task/restored-relation-catalog-readiness'
- [`7b0a804`](https://github.com/yelog/lazydb/commit/7b0a804115c36c585df427ad9b1ee8e5262c7964) fix(relation): wait for catalog before restoring tabs
- [`f049aad`](https://github.com/yelog/lazydb/commit/f049aad7845c30703bda88dd5dfea0005a4aed3e) fix(ci): align UI and catalog test contracts
- [`3271696`](https://github.com/yelog/lazydb/commit/3271696621ef68117673aff44f90d9a4c8571dd2) fix(ci): stabilize full codebase integration
- [`10a314c`](https://github.com/yelog/lazydb/commit/10a314c517d832c4a70af44640a3b6e4ebeb838b) feat(ui): complete interaction and help updates
- [`dbd607e`](https://github.com/yelog/lazydb/commit/dbd607ec9ac1a750fd25e1beac62ad785bb94507) merge: improve visible object picker feedback
- [`0d90811`](https://github.com/yelog/lazydb/commit/0d9081145d21ad7c88c7e15e076bf72c7b8f14b8) feat(ui): improve visible object picker feedback
- [`406a167`](https://github.com/yelog/lazydb/commit/406a167bf14f763d63ad4b2dde275e02eb814f2a) feat(ui): add help and viewport interaction updates
- [`3c47b2e`](https://github.com/yelog/lazydb/commit/3c47b2e9714a9e9939247c1e7d8341ade8575dbe) fix(mouse): scroll grid and explorer viewports immediately
- [`ec8abad`](https://github.com/yelog/lazydb/commit/ec8abadf31ecc56e930e4849e20c0297562adb05) fix(postgres): match structured catalog child paths
- [`c62a472`](https://github.com/yelog/lazydb/commit/c62a47280d7708b2f01d3f2b471c9ae7ac9823b1) fix: document alternate help shortcut
- [`88ba8c3`](https://github.com/yelog/lazydb/commit/88ba8c325cc2f4efb673621b7ee9c48569ace52a) fix(postgres): match full catalog search paths
- [`0af4738`](https://github.com/yelog/lazydb/commit/0af47384ae1472fb3962cd718e9ead0756329f3e) fix(postgres): prioritize qualified object suffixes
- [`b5deee1`](https://github.com/yelog/lazydb/commit/b5deee1c25ff13e6f64473eb7686f863162d89c7) docs: design contextual help shortcut
- [`e86925c`](https://github.com/yelog/lazydb/commit/e86925c6abb0207b30389d0b1a9e753b21589582) fix(postgres): prioritize qualified catalog matches
- [`c55daff`](https://github.com/yelog/lazydb/commit/c55daffabe7f254a9b4a33542628b4ced195cda3) fix(postgres): keep catalog search path scoped
- [`e4b7752`](https://github.com/yelog/lazydb/commit/e4b7752be138ed7202b4fe4ef27c6c11b37e24c6) fix(postgres): match qualified catalog searches
- [`8cdb6b6`](https://github.com/yelog/lazydb/commit/8cdb6b6f700fdfede7d93d61362225d514d104fd) fix(postgres): tolerate catalog output variations
- [`a8614d2`](https://github.com/yelog/lazydb/commit/a8614d26edab79134582fa5362c0ee768027936f) docs: restructure README for user onboarding
- [`39bc211`](https://github.com/yelog/lazydb/commit/39bc211ed5bd923379b219f49024c069c94e99b5) fix(postgres): exclude trigger functions from catalog
- [`065d69f`](https://github.com/yelog/lazydb/commit/065d69fd3edb6853db29bc31ca3941180c5aeac5) docs: add coding agent database access plan
- [`250298e`](https://github.com/yelog/lazydb/commit/250298e638f29b6a19760843e49bd5d8c0fe1fd5) merge: add coding agent database access
- [`58633ee`](https://github.com/yelog/lazydb/commit/58633ee20b2f4ccd77d724cdaea76a3ee0138f4f) fix(ui): highlight SQL in DDL and output logs
- [`9e79cd7`](https://github.com/yelog/lazydb/commit/9e79cd73c08e1d3c13a1f29312e179487fc296f0) fix(input): refresh pending key sequence timeout
- [`85fd406`](https://github.com/yelog/lazydb/commit/85fd406c11199cd40802aaca9befcbbc3ee14809) test(agent): harden database access boundaries
- [`4978187`](https://github.com/yelog/lazydb/commit/4978187002b6961dae31a15c84dd7478879dd94e) docs: explain coding agent database access
- [`204cd55`](https://github.com/yelog/lazydb/commit/204cd55e138fd8dd297e698254fa1ea08ee76b3d) feat(mcp): serve project-scoped database tools
- [`d824f6e`](https://github.com/yelog/lazydb/commit/d824f6ef927b2d46d9f21d07002006ac90a10375) feat(cli): add coding agent database commands
- [`1b01284`](https://github.com/yelog/lazydb/commit/1b01284b012f0732e33e68d7daa3edd8128ef73d) feat(agent): expose progressive schema inspection
- [`48c03f8`](https://github.com/yelog/lazydb/commit/48c03f81731e4f07075e649e41e8975cf9a8ee51) feat(agent): add headless database service
- [`f80e684`](https://github.com/yelog/lazydb/commit/f80e684ed3d4620eefd8ef4a8be4a59c2cbf683f) feat(agent): define API and write policy
- [`3f97180`](https://github.com/yelog/lazydb/commit/3f971801433782f2ad8131c4fc4dd69fd12f2259) merge: add motion-aware UI feedback
- [`d51c406`](https://github.com/yelog/lazydb/commit/d51c406bb2e0e6691274892ca0b7c542a81f8633) feat(ui): add motion-aware loading feedback
- [`a1599a6`](https://github.com/yelog/lazydb/commit/a1599a66818b8d2b81f4fc4604b2707d3b0a46d8) refactor(credentials): share headless profile resolution
- [`5842036`](https://github.com/yelog/lazydb/commit/5842036c9a6a55d79d1fdbfe06255a22adfc9b29) feat(agent): select connections deterministically
- [`396c0d9`](https://github.com/yelog/lazydb/commit/396c0d9e46179b2db926dc801b603a790483cb60) feat(agent): resolve project-visible connections
- [`5678d27`](https://github.com/yelog/lazydb/commit/5678d279a534b63e9540cdb28713186afe4e8f05) merge: add vim-style pane resizing
- [`f9bb215`](https://github.com/yelog/lazydb/commit/f9bb2152b2b712b62764f06f28cbd157ecacfee5) feat(layout): add vim-style pane resizing
- [`f69118c`](https://github.com/yelog/lazydb/commit/f69118c25604d6ef557aa7d84243fe67cb327e49) docs: simplify binary installation instructions
- [`de110f9`](https://github.com/yelog/lazydb/commit/de110f975883ae57ea1563d8435127c327d15904) merge: stabilize SQL completion popup position
- [`de3d7a7`](https://github.com/yelog/lazydb/commit/de3d7a79d821d37734a68600973e84069534c316) fix(ui): stabilize SQL completion popup position
- [`5273250`](https://github.com/yelog/lazydb/commit/527325035170c82c926e09501dc14d7c76620e79) docs: add implementation plans
- [`56c78c9`](https://github.com/yelog/lazydb/commit/56c78c9d63b0bd42689ab584c8de53245cb2d83e) feat: add read-only vim copy views
- [`34f8271`](https://github.com/yelog/lazydb/commit/34f8271cbf6bb06306aec4b3bfa28689b355f9a0) refactor(neovim): move plugin to standalone repository
- [`be20242`](https://github.com/yelog/lazydb/commit/be20242c77a4b48897af98d45f60fe848fbb417e) docs(neovim): update extraction execution plan
- [`87fb8b5`](https://github.com/yelog/lazydb/commit/87fb8b5a98b72d280c01e4317d42e4098c0e9e17) docs(neovim): preserve filtered extraction history
- [`6cf7b18`](https://github.com/yelog/lazydb/commit/6cf7b183777fa7f3eceab371830dddee25589197) docs(neovim): design standalone plugin extraction
- [`e790999`](https://github.com/yelog/lazydb/commit/e790999d18515042f3e0ca187419372b1c82c87b) fix(record-view): highlight selected field
- [`1a3f37f`](https://github.com/yelog/lazydb/commit/1a3f37f4b94fc3f9bacc78da11a1fc4b9d79c725) merge: integrate project-scoped connections
- [`b84fba9`](https://github.com/yelog/lazydb/commit/b84fba96f3deebea9d27debf8fcefa42457842ea) docs(neovim): document current repository installation
- [`c91da18`](https://github.com/yelog/lazydb/commit/c91da182b4f4a4d2b0eb14fae509250cdcabdb47) fix(clipboard): handle shifted row copy keys
- [`555a2fa`](https://github.com/yelog/lazydb/commit/555a2fa0ebc8f36bb256ce84f684c4709916951c) feat: show active connection in others group
- [`7384eb9`](https://github.com/yelog/lazydb/commit/7384eb9202bf894795d472bd75fac1d3f06039dd) docs: explain project-scoped connections
- [`a56ca2d`](https://github.com/yelog/lazydb/commit/a56ca2d95a73974de2749fdf3a8a8b3e72ce2e37) feat: reveal scoped startup connections
- [`5c4f9b3`](https://github.com/yelog/lazydb/commit/5c4f9b3aa50e890ac46bc39122a205f61252290d) feat: render project-aware connections
- [`6146471`](https://github.com/yelog/lazydb/commit/6146471f3679e0c2e381469e8f2e8a0f8c76a9e2) feat: add connection access menu
- [`466ae3d`](https://github.com/yelog/lazydb/commit/466ae3d661aa2d92a256069c72052cdfbeac4ba9) feat: update connection access transactionally
- [`ef08634`](https://github.com/yelog/lazydb/commit/ef08634bfa6ce4e9ffe9ccb52b6e6170664cb277) feat: scope new connections to current project
- [`6dd733e`](https://github.com/yelog/lazydb/commit/6dd733e16c1e3eced12695500280805d26fbe68b) feat: group unrelated connections in explorer
- [`5b926e1`](https://github.com/yelog/lazydb/commit/5b926e17decc85e1dd3887b1e0a17453447d9971) feat: pass project context into app
- [`8e582a9`](https://github.com/yelog/lazydb/commit/8e582a95b2ddf2d2af076dfee2a548d4e7cd0d9e) feat: persist connection access scope
- [`1d8b1b5`](https://github.com/yelog/lazydb/commit/1d8b1b5c7dc5b3c2f0ae119cf4a4d1242e3e4208) feat: resolve current project context
- [`6b31778`](https://github.com/yelog/lazydb/commit/6b3177886c8e6bc46f6b1d0db4e49b717d4f3d2e) merge: integrate trailing partial grid column
- [`0f93d2b`](https://github.com/yelog/lazydb/commit/0f93d2b59c8afde095e5590780c0b0274c15f0c1) merge: integrate context-aware copy
- [`49742a8`](https://github.com/yelog/lazydb/commit/49742a890ffba22f5614cd5a091ab017cca52ddf) feat: add per-connection workspaces
- [`52be7a2`](https://github.com/yelog/lazydb/commit/52be7a2e49b9d6aa2e49e72ea86760f680418d80) feat(clipboard): add context-aware copy actions
- [`778337e`](https://github.com/yelog/lazydb/commit/778337eebf89d15ee7f4384d79e209854689ee5c) Merge branch 'task/keyboard-navigation'
- [`29bd791`](https://github.com/yelog/lazydb/commit/29bd79107de5660fd5a4505f70cda36682463b5d) fix(ui): render trailing partial grid column
- [`a870c51`](https://github.com/yelog/lazydb/commit/a870c5106194ead0deed49c80cce54d8bea5194d) Merge branch 'main' of github.com:yelog/lazydb
- [`1d56695`](https://github.com/yelog/lazydb/commit/1d56695f890e5719a91d3691b7de27e509abe692) chore(deps): bump actions/checkout to v7.0.1
- [`6c3d4d0`](https://github.com/yelog/lazydb/commit/6c3d4d02334bce22109cf2c0b6f457a0a30116fa) chore(deps): bump actions/upload-artifact to v7.0.1
- [`cd44e2c`](https://github.com/yelog/lazydb/commit/cd44e2c2111922c83dc59d7d302f12b19c5994d7) chore(deps): bump actions/download-artifact to v8.0.1
- [`39a550b`](https://github.com/yelog/lazydb/commit/39a550b9e2a79fe68229b7e3ae870439026a6eca) chore(deps): bump actions/attest-build-provenance to v4.2.2
- [`970410e`](https://github.com/yelog/lazydb/commit/970410eb3066ebcb16d5750457039466b54c071e) chore(deps): bump actions/cache to v6.1.0
- [`fcdd45d`](https://github.com/yelog/lazydb/commit/fcdd45d41f6b7b87cc2180e949a4d820d6c1886a) ci(release): use current Intel macOS runner
- [`81393e3`](https://github.com/yelog/lazydb/commit/81393e327ab8bc94bd6caa16b75c83dc3f8c4db9) chore(release): prepare v0.1.0-beta.1
- [`167c8e6`](https://github.com/yelog/lazydb/commit/167c8e6b0653a585d7089404468cba601530ff4e) fix(mysql): preserve DDL child ordering
- [`2aec7aa`](https://github.com/yelog/lazydb/commit/2aec7aab2fc8f8fee9d9d0aad192d60fe1658b18) fix(mysql): preserve relation scope identity
- [`0407625`](https://github.com/yelog/lazydb/commit/0407625714ab1696039552bdd4cfcfb9260b5291) fix(workspace): address per-connection regressions
- [`c470a4e`](https://github.com/yelog/lazydb/commit/c470a4ed1063d405abb1f71f423d74e98cd18614) docs(workspace): explain per-connection tab restoration
- [`17757a1`](https://github.com/yelog/lazydb/commit/17757a1be9906fcc78acda4c59433ebea04aa054) fix(mysql): scope DDL children to verified database
- [`ff2592d`](https://github.com/yelog/lazydb/commit/ff2592d61f0be48bfefab5c3c26b349933268d5e) feat(workspace): delete workspace with its profile
- [`eeb88cb`](https://github.com/yelog/lazydb/commit/eeb88cbcd302f8cc8d7bba0d2a680a5f06d07854) fix(mysql): align DDL with requested catalog scope
- [`49fcb9b`](https://github.com/yelog/lazydb/commit/49fcb9b22d0030d4e6bd9b6fd90aeca74f154349) fix(workspace): scope console lifecycle by profile
- [`ed79fdd`](https://github.com/yelog/lazydb/commit/ed79fdd3f78a4be050077e41db12c4b3d18137df) feat(workspace): restore relation tabs lazily
- [`6ce1d32`](https://github.com/yelog/lazydb/commit/6ce1d32bfa6a0a90d5980781e3229c15620905db) fix(mysql): reuse verified relation DDL identity
- [`c16b76c`](https://github.com/yelog/lazydb/commit/c16b76c6eb595330f701c86428b28b33549958ff) feat(workspace): hide tabs when a profile disconnects
- [`3862160`](https://github.com/yelog/lazydb/commit/3862160460555a0316221399a3330e52f3e532d8) fix(mysql): compare mirrored schemas canonically
- [`e65cd2f`](https://github.com/yelog/lazydb/commit/e65cd2f8f94ee52fbb07b5db737f840f8955613f) fix(mysql): canonicalize relation schema lookup
- [`0fe15e0`](https://github.com/yelog/lazydb/commit/0fe15e05494e5682825cafb3435eddbe6c09c600) fix(workspace): guard connection switches against live work
- [`5de3200`](https://github.com/yelog/lazydb/commit/5de32004551a0709c7cc239fa8d616783582cd42) fix(mysql): honor canonical name mode
- [`3a143cc`](https://github.com/yelog/lazydb/commit/3a143cc8c6d79340a0183c6a12771846a1117059) feat(workspace): swap tabs after successful connection
- [`9fb8aa9`](https://github.com/yelog/lazydb/commit/9fb8aa950dcb42930bb34491032da8fdf4186ff3) fix(mysql): normalize system variable decoding
- [`db8f7c3`](https://github.com/yelog/lazydb/commit/db8f7c3cac2d0e8f67e25d18de96684a20cfd6cd) feat(workspace): snapshot and restore every profile workspace
- [`44d70cf`](https://github.com/yelog/lazydb/commit/44d70cfe91b3d29bf0b818b329f05d162a0e9faf) fix(ci): finish integration compatibility checks
- [`040540f`](https://github.com/yelog/lazydb/commit/040540fedf561a55a6ec65d1bbf18b6127379015) feat(workspace): migrate flat workspaces by profile
- [`81cf7bc`](https://github.com/yelog/lazydb/commit/81cf7bcda8fa42ff806b2328f20a20f555d00759) feat(workspace): define profile-scoped persistence format
- [`718648d`](https://github.com/yelog/lazydb/commit/718648d866ee392c4299d11bdfbf318555354daf) feat: improve keyboard navigation
- [`0a5cd2e`](https://github.com/yelog/lazydb/commit/0a5cd2e05689e4cfc1dc73f1b2cf061b9c993ed1) fix(ci): stabilize cross-platform release checks
- [`7c5df07`](https://github.com/yelog/lazydb/commit/7c5df07f54ec6b559df52ed1d71ccf6bb3463eff) refactor(workspace): add profile-scoped workspace state
- [`bd699b4`](https://github.com/yelog/lazydb/commit/bd699b409054139cf75f04bc5e9d375acea8be1e) docs: design keyboard navigation
- [`71caf64`](https://github.com/yelog/lazydb/commit/71caf64cd27519b2bedbb1de1d04a5beaa62be2f) fix(mysql): avoid dynamic catalog column names
- [`9e64682`](https://github.com/yelog/lazydb/commit/9e64682d8ee897858a882ec561725e02882524da) fix(mysql): read catalog metadata by column position
- [`a0563b4`](https://github.com/yelog/lazydb/commit/a0563b418eb953d2813dedb661b3a2f267e61cbb) fix(ci): finish platform compatibility fixes
- [`eae3459`](https://github.com/yelog/lazydb/commit/eae3459a8c2399d13afbeb6bee4dd8cbae9bad22) fix(ci): handle current MySQL metadata and lints
- [`106a828`](https://github.com/yelog/lazydb/commit/106a828a8a100cfb9c8cd2d7f31ceb80f80a79cd) fix(ci): restore release quality gates
- [`2f24338`](https://github.com/yelog/lazydb/commit/2f24338922d80d51098b8462d750c3d20e478dc2) feat(tabs): replace sequence numbers with context icons
- [`30cb1d2`](https://github.com/yelog/lazydb/commit/30cb1d264eb3452151cccb5770e9093305cbf287) fix(ci): support current release runner dependencies
- [`d4d99d7`](https://github.com/yelog/lazydb/commit/d4d99d754c8799ccc8ca5c59bebc7ba17d844d2d) ci(release): add beta and stable distribution pipeline
- [`f42ae8a`](https://github.com/yelog/lazydb/commit/f42ae8aab9240f68df0a140e1f1f27c4445bdb8e) fix(results): accept data query completion with enter
- [`2c7ba09`](https://github.com/yelog/lazydb/commit/2c7ba094965b8e3ca0665bf7a0d0cfd6fc144e9f) fix(grid): keep row numbers muted
- [`9d7d47d`](https://github.com/yelog/lazydb/commit/9d7d47dc4f1b4693c116fac77505ba3640ac7eb7) Merge task/identifier-completion into main
- [`b69385b`](https://github.com/yelog/lazydb/commit/b69385b2f7187dc8b3e076f3627d0bfc5c3b48b8) feat(sql): add fuzzy identifier completion
- [`50b05a9`](https://github.com/yelog/lazydb/commit/50b05a9b272563a658b3b2be224b9f118238bc12) docs: add identifier completion plans
- [`f3459b4`](https://github.com/yelog/lazydb/commit/f3459b4cb1f080fc8296aa06825778fa44652ea9) feat(results): add read-only record view
- [`ece8f8b`](https://github.com/yelog/lazydb/commit/ece8f8b50091584e4fabdfd144c50634d0620b55) fix(grid): remove header separator row
- [`047da11`](https://github.com/yelog/lazydb/commit/047da11c52c200b204c5b7757bf6dbcd5412a7b4) fix(grid): clean up empty result rendering
- [`2f96f9c`](https://github.com/yelog/lazydb/commit/2f96f9c4c8cebd2dc75ae9377295ed479bf6ad51) fix(connection): enforce ten-second connect timeout
- [`e262147`](https://github.com/yelog/lazydb/commit/e2621479daa723540fb567203e32aed450da9337) fix(explorer): keep parent selection visible
- [`6219951`](https://github.com/yelog/lazydb/commit/6219951d83f8cda37eab826d043df3cdb431229d) feat(explorer): pin selected ancestor rows
- [`1295bf0`](https://github.com/yelog/lazydb/commit/1295bf0f8ef12bf856f2ae834d418b700fb147e9) feat(sql): cap ad-hoc select results at 500 rows
- [`b8e52e7`](https://github.com/yelog/lazydb/commit/b8e52e7314b2349781f01673e30ec521c680081a) feat(explorer): improve search start and metadata display
- [`c91993f`](https://github.com/yelog/lazydb/commit/c91993f47aa21a103693212f33eac8b42ea2cee0) feat(input): change quit shortcut to Ctrl+C
- [`479d226`](https://github.com/yelog/lazydb/commit/479d2262a1ba1f2f65d8284959867640b4683415) fix(editor): refine insert controls and completion lifecycle
- [`02cdb61`](https://github.com/yelog/lazydb/commit/02cdb61007c1da37714b0873b98a21d022234de1) feat(explorer): move catalog search to frontend
- [`5c71155`](https://github.com/yelog/lazydb/commit/5c7115579ee1800bd5c560c78ee04bf405bd153b) feat(input): unify single-line editor controls
- [`67e9a3c`](https://github.com/yelog/lazydb/commit/67e9a3c40969fbbcf305e0439030dbaea10cea12) fix(explorer): show column type in details
- [`07e13b9`](https://github.com/yelog/lazydb/commit/07e13b93bfdc1390eb2fc57516bc8716376c6862) fix(explorer): restore navigation after find confirmation
- [`332baa3`](https://github.com/yelog/lazydb/commit/332baa3e1a98cdd815e96f24a4e10dbce42ad3c8) fix(ui): route DDL window focus commands
- [`50c4acb`](https://github.com/yelog/lazydb/commit/50c4acb29947dbd0731a5faa7f5ea0076a15e3e5) fix(explorer): adapt find selection to viewport state
- [`99f5997`](https://github.com/yelog/lazydb/commit/99f5997a6f47d965b82ad472b5fd349ce6d8bd85) merge: integrate Explorer dual search
- [`493cf0d`](https://github.com/yelog/lazydb/commit/493cf0d20712d81f3c6fd214f90f9aec073449df) feat(explorer): split visible and catalog search
- [`ce77866`](https://github.com/yelog/lazydb/commit/ce77866141ded383c9446c2c39d098bf88f7c345) Merge branch 'task/relation-ddl'
- [`1a22f00`](https://github.com/yelog/lazydb/commit/1a22f00fed89db7e38ef4cdd6fbaa1f044accce9) feat: add relation DDL preview
- [`a525c04`](https://github.com/yelog/lazydb/commit/a525c04d1d3ed8ff6fffd918b3ebfa610b397932) feat(explorer): improve tree navigation and column ordering
- [`d59f80b`](https://github.com/yelog/lazydb/commit/d59f80b23500b9530c03bb7249a4c6647fc0c8de) fix(ui): refresh filtered relation rows
- [`c64f18e`](https://github.com/yelog/lazydb/commit/c64f18e0feecac99c45d86090937633c14b8365c) feat(ui): add Vim data grid navigation
- [`1230bdf`](https://github.com/yelog/lazydb/commit/1230bdf4b0615b5b8b7e3e734827075393437c60) fix: preserve explorer search key routing
- [`43c8bf6`](https://github.com/yelog/lazydb/commit/43c8bf6f4e04e0779f8cc197de13d9b91e6f52c2) merge: integrate SQL editor lifecycle
- [`224f487`](https://github.com/yelog/lazydb/commit/224f487b5d78084d952dec1df5975bc49c5b88d9) feat: manage SQL editor lifecycle
- [`3d20edd`](https://github.com/yelog/lazydb/commit/3d20eddec312e99a776f303974e97c9f0225c23b) Merge branch 'task/table-preview'
- [`4c8aba4`](https://github.com/yelog/lazydb/commit/4c8aba4c47ee8694fac1f5f28bd5b7cb7a8c231b) fix(ui): keep grid selection visible while scrolling
- [`ecdfc48`](https://github.com/yelog/lazydb/commit/ecdfc48df43ad1c3fa9c82d4179a47102ba07965) merge: integrate relation pane focus navigation
- [`34c0f43`](https://github.com/yelog/lazydb/commit/34c0f439e2e4612e4444225815a6d7aa1dcc8dce) feat(explorer): add catalog object search
- [`6db1421`](https://github.com/yelog/lazydb/commit/6db14219521c6c70f31bb6d34ae7cc441323460e) fix(ui): restore relation pane focus navigation
- [`cbdaf69`](https://github.com/yelog/lazydb/commit/cbdaf697902a1e4103e845b37dca7a4a7c656641) fix: expand profile after manual connection
- [`09cd959`](https://github.com/yelog/lazydb/commit/09cd95927aaf9b24764410454b7fee6cd83b104e) fix(ui): improve table preview formatting
- [`d36630b`](https://github.com/yelog/lazydb/commit/d36630bd15676c962c9bd66be614045a8ad76f51) merge: integrate transactional relation data editing
- [`5240404`](https://github.com/yelog/lazydb/commit/52404049769d560bac4308b6a9bdb600f05b0649) feat: add relation mutation types
- [`f2a17f2`](https://github.com/yelog/lazydb/commit/f2a17f2ba8c205facc16d82b82cfd9c731692a2a) feat: add transactional relation data editing
- [`757f9cd`](https://github.com/yelog/lazydb/commit/757f9cd70faf8a14358d244a0383d34426822f3f) docs: add relation editing implementation plans
- [`a3e6fcb`](https://github.com/yelog/lazydb/commit/a3e6fcb1cd6367d1e41596acf4498c8f8cafd163) fix(sql): resolve statements from internal whitespace
- [`d8b2277`](https://github.com/yelog/lazydb/commit/d8b22777035f8d12e046c5731adf3f7d5a66d8ed) feat(ui): add horizontal data grid scrolling
- [`4e47002`](https://github.com/yelog/lazydb/commit/4e470028c0ae1fd0f852da5c81e7e9aeb5b87717) merge: integrate SQL editor completion and formatting fixes
- [`4b056f5`](https://github.com/yelog/lazydb/commit/4b056f5f446445004df2d4325965e3c95c413fc7) fix(sql): recover exit after lost transaction connection
- [`f07df25`](https://github.com/yelog/lazydb/commit/f07df25727f843c07bf0632babc4990fce4f5a47) test(ui): cover qualified completion popup rendering
- [`87af00f`](https://github.com/yelog/lazydb/commit/87af00f9e71efce8d9999cc74a452b614fe5542a) feat(sql): use editor target for completion
- [`7a86a5a`](https://github.com/yelog/lazydb/commit/7a86a5ab5f794087f9b956d942bb637c4084cd8d) feat(sql): qualify relation completion candidates
- [`44cb1cd`](https://github.com/yelog/lazydb/commit/44cb1cdb7e214909e5c120f58c18cccccd84da55) feat(sql): expose selected and current formatting
- [`b695fa4`](https://github.com/yelog/lazydb/commit/b695fa48602fdf1410bd4df63793cc8e286cd0b8) fix(sql): restrict completion to insert mode
- [`89c29b6`](https://github.com/yelog/lazydb/commit/89c29b6c3f942dd8b5c118643f68b0b873b175ab) docs(sql): design execution output log
- [`deb1fb1`](https://github.com/yelog/lazydb/commit/deb1fb105af871855a337167ddde4e6390b66056) docs(sql): plan completion and formatting fixes
- [`1e33287`](https://github.com/yelog/lazydb/commit/1e33287b72f41000cb7cfeaa17d2431001c9ae09) fix(sql): handle transaction toggle shortcut
- [`4a093ff`](https://github.com/yelog/lazydb/commit/4a093ff4b69d97d11fb8a13c74b87b2a078d8c40) docs(sql): design completion and formatting fixes
- [`41345ed`](https://github.com/yelog/lazydb/commit/41345ed7d54e40f71ccecf193984f7d18d1029e5) merge: integrate searchable help palette
- [`6d70c2a`](https://github.com/yelog/lazydb/commit/6d70c2a5d47c99ee970edf6fd0195bf0e35eb15f) feat(help): add searchable shortcut palette
- [`8682ef4`](https://github.com/yelog/lazydb/commit/8682ef4b46404492399b91cb150e99103f9026f1) merge: integrate fixed connection URL section
- [`d4fa4ac`](https://github.com/yelog/lazydb/commit/d4fa4acd0a78c9acec6c6f1da2d67e715051c194) feat(ui): move connection URL to fixed form section
- [`6560e64`](https://github.com/yelog/lazydb/commit/6560e64cd1a669e64007b145cf426f4d5242a70c) feat(sql): unify result data grid and filtering
- [`7b4c01e`](https://github.com/yelog/lazydb/commit/7b4c01ef75b4fa2aaa9630b57d57f18475f5cd7d) feat(ui): add icons to profile driver options
- [`4af5269`](https://github.com/yelog/lazydb/commit/4af52693dad2582d99fb1aba66e4184e64a094ed) fix(ui): prevent pane focus flicker
- [`fdf04c0`](https://github.com/yelog/lazydb/commit/fdf04c086ef7b6037c037cbd32ee4fa096c213a8) fix(sql): sync cursor style with editor mode
- [`a01734c`](https://github.com/yelog/lazydb/commit/a01734c98a330fe9e995b59da6596c8bd8afd5f7) merge: integrate SQL editor completion assistance
- [`d172e04`](https://github.com/yelog/lazydb/commit/d172e0456c0ec215b1e08161a29612fda1bc23f9) feat(sql): improve editor completion assistance
- [`d7676cc`](https://github.com/yelog/lazydb/commit/d7676cc36bd9561c6eb3e80fe35d08a00dfdb325) feat(credentials): add local encrypted password storage
- [`3264f0b`](https://github.com/yelog/lazydb/commit/3264f0b73c69fd44fbcbeb982bdbb6f01157c394) style(ui): refine explorer hierarchy
- [`8c6c266`](https://github.com/yelog/lazydb/commit/8c6c266f75b07c427f2545d42c450d093dd90008) fix(profiles): stabilize visible object selection
- [`ffdcc1f`](https://github.com/yelog/lazydb/commit/ffdcc1f564b3378f2819d218bc332332d5d5fcf2) feat(profiles): discover visible objects automatically
- [`afc366e`](https://github.com/yelog/lazydb/commit/afc366e8cc0ed5fdf922551ceac3c8c1f170850c) feat(ui): add configurable terminal icons
- [`a192121`](https://github.com/yelog/lazydb/commit/a192121a6e290412d9aef17cc7a6f04b8e933983) fix(ui): align workspace tabs with main content
- [`a03daeb`](https://github.com/yelog/lazydb/commit/a03daeb7bfd9c89fb927ccd6ff16cfe41b3fb06d) chore: ignore codegraph index
- [`92c24fb`](https://github.com/yelog/lazydb/commit/92c24fb6ceb3a74427c04b1d4a0f2a2389347b76) fix(keymap): preserve leader shortcuts on relation tabs
- [`9b7a32a`](https://github.com/yelog/lazydb/commit/9b7a32a3f01b279a6e3ba91712cc47091bbf639d) fix(explorer): respect focus for relation shortcuts
- [`813a764`](https://github.com/yelog/lazydb/commit/813a76419728a9872f13d8bfc9bd5c2312e7d1b9) fix(profiles): persist new connection passwords by default
- [`fb674d7`](https://github.com/yelog/lazydb/commit/fb674d7d33088a4b3dde16e0b99c63264c854bb7) merge: integrate relation preview controls
- [`4c26bfa`](https://github.com/yelog/lazydb/commit/4c26bfa89eadef04381230e5b001881fd89e82e5) feat(relation): improve data preview controls
- [`d5207be`](https://github.com/yelog/lazydb/commit/d5207befb98476448b957d3d1d6b5d5f3facb347) feat(editor): add execution target selector
- [`3adb069`](https://github.com/yelog/lazydb/commit/3adb069a0a8ae72379358de5b5b789c8f1107c26) feat(profiles): improve connection management
- [`33c6928`](https://github.com/yelog/lazydb/commit/33c6928dba64250713b6230acf3bcb4e36fb0664) merge: integrate sqleditor worktree into main
- [`942e729`](https://github.com/yelog/lazydb/commit/942e7296d4dbe42abb8926ba94293ebe090916cc) feat(explorer): complete database explorer and relation pages
- [`0c3513e`](https://github.com/yelog/lazydb/commit/0c3513e6d98250ed4148238f0dd1d8b9178d8088) feat(ui): move editor context into title
- [`4205592`](https://github.com/yelog/lazydb/commit/42055923a4cd67227681c7be95d7c1f3a5396813) docs(ui): plan editor context title
- [`cc1270a`](https://github.com/yelog/lazydb/commit/cc1270a2a286b71bae60d32f073d5c7c04749daf) docs(ui): design editor context title
- [`4d8b1d4`](https://github.com/yelog/lazydb/commit/4d8b1d4addcbc2251595c5b19eabbf9caef684e2) fix(completion): preserve accepted cursor lifecycle
- [`3ad92cf`](https://github.com/yelog/lazydb/commit/3ad92cfa714b6bcddea37565bcaaf3a0aa66179a) docs(completion): plan accept cursor lifecycle
- [`a2eece1`](https://github.com/yelog/lazydb/commit/a2eece1affaa0ec483ef0de7d3bf7fda06eb7b3b) docs(completion): design accept cursor lifecycle
- [`a1105eb`](https://github.com/yelog/lazydb/commit/a1105ebea17947b12ffec3dad932957303cc38e1) fix(editor): preserve global keys and completion boundaries
- [`a674ad7`](https://github.com/yelog/lazydb/commit/a674ad7df18bd1739b0b3ae2162cee1126becfff) docs(editor): plan keymap completion lifecycle
- [`5253eeb`](https://github.com/yelog/lazydb/commit/5253eeb1b2945e5ad82488f8442afbdb24483317) docs(editor): design keymap completion lifecycle
- [`aef8bba`](https://github.com/yelog/lazydb/commit/aef8bba7565327b22c7ed14ec20ae2e012e9ad40) feat(workspace): debounce saves and configure schema
- [`e6c3040`](https://github.com/yelog/lazydb/commit/e6c3040d309295840be97ed3f0fbb06d0fab8cf9) feat(sql): add connection target selector
- [`4f598de`](https://github.com/yelog/lazydb/commit/4f598de8fe2a9c1b009ca3d4d501e51eb3950561) fix(workspace): add single-writer lock and durable saves
- [`6506aef`](https://github.com/yelog/lazydb/commit/6506aef0c6cc45ba2aff93829f9d49ef2fe777b9) feat(workspace): load restored consoles at startup
- [`64ede18`](https://github.com/yelog/lazydb/commit/64ede183519c6d916a3121d965c587672440e2d4) fix(workspace): keep targets aligned after profile changes
- [`cab487f`](https://github.com/yelog/lazydb/commit/cab487f2befa0efe5819a487c8086663463c49df) feat(workspace): persist sql editor snapshots
- [`9f5e34b`](https://github.com/yelog/lazydb/commit/9f5e34b969482f8ef124d71bb8a6ba3446970cd9) feat(sql): switch connections with editor tabs
- [`8cdf5ce`](https://github.com/yelog/lazydb/commit/8cdf5ce0f7ccd35062f46e1f222d6867557253a1) feat(sql): add target selection commands
- [`fd5048f`](https://github.com/yelog/lazydb/commit/fd5048f9baca409a0ce2ee558012f024a4cea023) feat(sql): add editor execution targets
- [`bb3bcd0`](https://github.com/yelog/lazydb/commit/bb3bcd0c4f1a9a888559010fae5fe583cdc05564) feat(transaction): expose editor controls
- [`6e3f47d`](https://github.com/yelog/lazydb/commit/6e3f47d96ce9767e3ed34a1f5de1d6d037b6314f) fix(transaction): own cancellation and shutdown cleanup
- [`8b5b1ef`](https://github.com/yelog/lazydb/commit/8b5b1ef20e42b1c7edc5c20eb3457604f9105645) fix(transaction): reject stale worker commands
- [`2c6df8a`](https://github.com/yelog/lazydb/commit/2c6df8ad0b511c32d180aeb9dbd6d4af60a4a72a) fix(transaction): await begin and retire workers
- [`36f925a`](https://github.com/yelog/lazydb/commit/36f925adb42bd602c323c3b58a3c367c409b1562) fix(transaction): honor exit choices and scoped sql
- [`c208316`](https://github.com/yelog/lazydb/commit/c2083166ec320bfcd7819864b3c0eef15e1dca9f) feat(editor): add cursor styles and selection rendering
- [`7e086d2`](https://github.com/yelog/lazydb/commit/7e086d2589b39fb5736ca496ec7f3272b831f1dc) fix(editor): route vim input through console machines
- [`cb155e9`](https://github.com/yelog/lazydb/commit/cb155e9864439e783be778dc1be898e2b7e37f5d) test(editor): characterize modal input failures
- [`044e201`](https://github.com/yelog/lazydb/commit/044e2019f7ab13fd86b26719bcc8d008f4c6a8fd) feat(explorer): implement database explorer
- [`a7d3ebb`](https://github.com/yelog/lazydb/commit/a7d3ebbded4516be5232cbc9a1f7b628a133d0af) docs(sql): plan editor runtime context
- [`a4cfd0d`](https://github.com/yelog/lazydb/commit/a4cfd0d7439cacc579df8c94431677dd6ee77b0e) docs(sql): design editor runtime context
- [`ee3270e`](https://github.com/yelog/lazydb/commit/ee3270ee81fcb160ce771bbbbd8bb2c99eb29f74) feat(sql): add editor transactions and completion
- [`713ed1e`](https://github.com/yelog/lazydb/commit/713ed1ed0d98f92713c8124067cfcca107295547) docs: document dynamic connection profiles
- [`d720c2e`](https://github.com/yelog/lazydb/commit/d720c2e441e29e75eaa624211c5fe9acb1ab1694) test(profiles): cover full lifecycle
- [`b71bd7d`](https://github.com/yelog/lazydb/commit/b71bd7d1fd636a7544e8adf078e1d2c5788f88ec) feat(profiles): open manager on first launch
- [`ccdc313`](https://github.com/yelog/lazydb/commit/ccdc31301d50750e57462b7ca75eb49061d735a2) feat(ui): render connection profile manager
- [`aaab930`](https://github.com/yelog/lazydb/commit/aaab9300ab1723564d6e7c029f226341ff28e116) feat(profiles): add manager input controls
- [`1c32c4d`](https://github.com/yelog/lazydb/commit/1c32c4db633af15b2a1b1ede503a790656f5b39d) fix(runtime): bind commands to active connections
- [`4fc1ab1`](https://github.com/yelog/lazydb/commit/4fc1ab11079df71a7b381a7fa77fbcca630c893b) feat(profiles): persist runtime profile changes
- [`c37cf1f`](https://github.com/yelog/lazydb/commit/c37cf1f1e1f312b8caca6154bd71c7db921241f3) feat(profiles): add profile manager reducer
- [`cdce725`](https://github.com/yelog/lazydb/commit/cdce725827873e63d9ce030f0385faa7884f8f2d) feat(profiles): add connection draft validation
- [`bb5e9e4`](https://github.com/yelog/lazydb/commit/bb5e9e47811c30d10670fb90b1917f0909d7a94c) feat(security): add native secret store boundary
- [`20bd92c`](https://github.com/yelog/lazydb/commit/20bd92ca86d306a216d11603d817d0e31639c4c2) docs: plan dynamic profile manager implementation
- [`b5de003`](https://github.com/yelog/lazydb/commit/b5de0034e04af05e67b247fb9c1314da2e776a08) docs: design SQL editor and transactions
- [`7b54b9c`](https://github.com/yelog/lazydb/commit/7b54b9cfdc608554e8e6023724c5888f69b5ecf9) docs: design dynamic profile manager
- [`074322d`](https://github.com/yelog/lazydb/commit/074322d4bfe3b126a98471ee89347b7453dd622a) feat: implement LazyDB M0 foundation

## [0.1.1] - 2026-09-09

### Added

- feat(help): add selectable Ctrl-c quit shortcut.
- feat(ui): review pending transactions before quit.
- feat(ui): review relation changes before commit.
- feat(ui): clarify table header sort states.
- feat(sql): diagnose missing tables and columns.
- feat(output): improve error diagnostics and styling.
- feat(lsp): add segment-aware catalog completion.
- feat(completion): distinguish semantic completion icons.
- feat(sql): complete update target columns.
- feat(sql): offer dialect builtin expressions in completion.
- feat(console): log target and transaction switches in OUTPUT.
- feat(uninstall): add safe native installation removal.
- Plus 32 additional related changes recorded below.

### Changed

- style(lsp): format catalog load logging.
- perf: bound query retention and reduce workspace copy overhead.

### Fixed

- fix(installer): verify Windows behavior and Pages deployment.
- fix(installer): support Windows PowerShell bootstrap.
- fix(installer): show current shell activation instructions.
- fix(runtime): allow relation loads before catalog warmup.
- fix(runtime): remove stale relation catalog admission.
- fix(catalog): reconcile renamed relations after SQL changes.
- fix(ui): improve transaction review layout and generate local SQL preview.
- fix(relation): preserve typed editors for new cells.
- fix(ui): simplify relation review predicates.
- fix(sql): complete ALTER COLUMN definitions.
- fix(ui): keep relation DDL cursor visible.
- fix(ui): focus query input content on mouse click.
- Plus 49 additional related changes recorded below.

### Internal

- Merge remote-tracking branch 'origin/main'.
- Enhance README with MCP and Neovim LSP details.
- test: avoid wildcard paths in Windows fixture logging.
- ci: fix Windows workflow shell matrix.
- ci: add manual workflow dispatch.
- test: align transaction panel assertions.
- Merge branch 'task/windows-installer-compatibility'.
- test: include builtin expressions in completion kinds.
- test: tolerate asynchronous profile lifecycle events.
- merge: fix stale relation catalog admission.
- Merge branch 'task/catalog-change-reconciliation'.
- merge: review pending transactions before quit.
- Plus 82 additional related changes recorded below.

### Commits

- [`361b1bc`](https://github.com/yelog/lazydb/commit/361b1bce3529d1147c76641e88ea901be203416a) Merge remote-tracking branch 'origin/main'
- [`5646407`](https://github.com/yelog/lazydb/commit/5646407a3c17cdcda79f236b90a06a5d4f322a62) feat(help): add selectable Ctrl-c quit shortcut
- [`bd7d75d`](https://github.com/yelog/lazydb/commit/bd7d75d39ec5e247fccc1725932f6cc9aa0ad962) Enhance README with MCP and Neovim LSP details
- [`3f5a629`](https://github.com/yelog/lazydb/commit/3f5a62930b157afbebce8b47c936cb9170c5a67f) test: avoid wildcard paths in Windows fixture logging
- [`620f93e`](https://github.com/yelog/lazydb/commit/620f93efc0aebabf7c862a85661448327f470759) fix(installer): verify Windows behavior and Pages deployment
- [`0d68bf7`](https://github.com/yelog/lazydb/commit/0d68bf7858c790e45df6d5c2119c88305eeb4d3e) ci: fix Windows workflow shell matrix
- [`b0f48e5`](https://github.com/yelog/lazydb/commit/b0f48e51401811ff1ea5311d4539dcaa52152b4a) ci: add manual workflow dispatch
- [`b8de84d`](https://github.com/yelog/lazydb/commit/b8de84d79c7752f332bfe1eebee850c1e5c30e21) test: align transaction panel assertions
- [`ad4c5ce`](https://github.com/yelog/lazydb/commit/ad4c5cebfc5311bbd0a5b6ba041ea7fcd2709a93) Merge branch 'task/windows-installer-compatibility'
- [`f0b41d9`](https://github.com/yelog/lazydb/commit/f0b41d94e1d845f23bc56ecd9cf32037166dab30) fix(installer): support Windows PowerShell bootstrap
- [`6044897`](https://github.com/yelog/lazydb/commit/60448975c1327177b0e85f218222cfb908dc2f0d) fix(installer): show current shell activation instructions
- [`85e05d8`](https://github.com/yelog/lazydb/commit/85e05d8808e59b07ea2f3d6a01d6470b2c121d67) test: include builtin expressions in completion kinds
- [`f2fdf37`](https://github.com/yelog/lazydb/commit/f2fdf3753fc3c4e3686a2303eb7de22f0a7cfbab) test: tolerate asynchronous profile lifecycle events
- [`80bef89`](https://github.com/yelog/lazydb/commit/80bef896090c67c07800c8a8a913183602224bfd) fix(runtime): allow relation loads before catalog warmup
- [`0ca1c09`](https://github.com/yelog/lazydb/commit/0ca1c096fbae6ab53460f25d78e21360bc845c8b) merge: fix stale relation catalog admission
- [`9f97621`](https://github.com/yelog/lazydb/commit/9f97621fe850dc17bb85090b0aab1c5b3d0bfefb) fix(runtime): remove stale relation catalog admission
- [`32fac1b`](https://github.com/yelog/lazydb/commit/32fac1bb4a634cb189c8f87e9630ef3f69a8bd91) Merge branch 'task/catalog-change-reconciliation'
- [`6f7f825`](https://github.com/yelog/lazydb/commit/6f7f82547d357e3ca8627d5bd8dd9edfc1ea1c45) fix(catalog): reconcile renamed relations after SQL changes
- [`8224da9`](https://github.com/yelog/lazydb/commit/8224da95fd8a2285d7bdc7ab1c578f7656752c6e) fix(ui): improve transaction review layout and generate local SQL preview
- [`da54e68`](https://github.com/yelog/lazydb/commit/da54e6882387cb92566c063180639c6ce90cb274) merge: review pending transactions before quit
- [`4de2e69`](https://github.com/yelog/lazydb/commit/4de2e69358469cbf4139922c5ae3b2feb8037386) feat(ui): review pending transactions before quit
- [`1ea5503`](https://github.com/yelog/lazydb/commit/1ea5503ead69ba2bba862c9b7a39ca44c4a343f8) chore: clean up merged relation editor lints
- [`8cdb256`](https://github.com/yelog/lazydb/commit/8cdb256449aac01533815018ac1a94f465b1a1eb) merge: preserve typed editors for new relation cells
- [`401676c`](https://github.com/yelog/lazydb/commit/401676c357efdbe39b96ac265b3adb618fd5d3f3) fix(relation): preserve typed editors for new cells
- [`6f544ad`](https://github.com/yelog/lazydb/commit/6f544ad97ff01b42fa634107408675029567886b) merge: simplify relation review predicates
- [`d8906c4`](https://github.com/yelog/lazydb/commit/d8906c4f94dcbeb72164a35a889de3bad90833c5) fix(ui): simplify relation review predicates
- [`4c74df1`](https://github.com/yelog/lazydb/commit/4c74df1e7230202b8e8b3990317dfe91e34b19b2) merge: complete ALTER COLUMN definitions
- [`96bbf45`](https://github.com/yelog/lazydb/commit/96bbf451951bc06fc2179f6a38bfd02a7689bfe3) fix(sql): complete ALTER COLUMN definitions
- [`0355f0f`](https://github.com/yelog/lazydb/commit/0355f0fc43dd12a3738a4d245eadb7160d631f57) Merge branch 'task/relation-ddl-navigation'
- [`835c26c`](https://github.com/yelog/lazydb/commit/835c26c6d8782fdc20de727e11b923596a14139a) fix(ui): keep relation DDL cursor visible
- [`9584b65`](https://github.com/yelog/lazydb/commit/9584b6508a73bf56fefcf751bd1c995aebadec6e) merge: review relation changes before commit
- [`da7ae6c`](https://github.com/yelog/lazydb/commit/da7ae6c767dfac97769df3046b8d4c09a5075ce0) feat(ui): review relation changes before commit
- [`ee14734`](https://github.com/yelog/lazydb/commit/ee1473423899d3e04512e64e866c94314c544aa1) merge: clarify table header sort states
- [`cac23d2`](https://github.com/yelog/lazydb/commit/cac23d20f706aeaf26c6ce01b13f8c50dc64c10f) feat(ui): clarify table header sort states
- [`7a2b485`](https://github.com/yelog/lazydb/commit/7a2b485e2444bee2854ca100be397f2cf8ec3b23) merge: fix query bar mouse focus
- [`00dc85b`](https://github.com/yelog/lazydb/commit/00dc85bfd4d7cef39e9a8d34a24658987f6640eb) fix(ui): focus query input content on mouse click
- [`80b118b`](https://github.com/yelog/lazydb/commit/80b118bdd2e539f57e9bd77dbc0f8e53c82eeb54) merge: restore console target connections
- [`63ede76`](https://github.com/yelog/lazydb/commit/63ede76701e3057c7d742c7c1fc48d610486fdf2) fix(sql): restore console target connections
- [`1d821fc`](https://github.com/yelog/lazydb/commit/1d821fc88eea4468d3d8c22de144c975e11aa566) merge: improve SQL editor completion consistency
- [`a487ad3`](https://github.com/yelog/lazydb/commit/a487ad3dc9d5269ccc5120a3de762a3e59753b74) fix(sql): improve editor completion consistency
- [`218228a`](https://github.com/yelog/lazydb/commit/218228a1d1d70ade48dde881639ae0ce9d6be0e0) merge: improve ALTER TABLE completion
- [`d8894f6`](https://github.com/yelog/lazydb/commit/d8894f640ee1302aae72c5e4a868aa580bab6ec4) fix(sql): improve ALTER TABLE completion
- [`04df7a3`](https://github.com/yelog/lazydb/commit/04df7a31cdbb87d35a41f3aca98423cf839e6a5d) fix(sql-editor): restore viewport sync while editing
- [`b179ecf`](https://github.com/yelog/lazydb/commit/b179ecf6bc1e34dec14f982f67338d8b4e9ea236) merge: diagnose missing SQL tables and columns
- [`def46cc`](https://github.com/yelog/lazydb/commit/def46cc99390e48b3c51afe3554008baa6d3690c) feat(sql): diagnose missing tables and columns
- [`53a7e66`](https://github.com/yelog/lazydb/commit/53a7e66eb1c08fe0afcafcb03100c2184d6cebe7) fix(sql): preserve embedded diagnostic source ranges
- [`06e94c4`](https://github.com/yelog/lazydb/commit/06e94c42d0c1dfb193e037dda5eb26c9d9911364) fix: remove duplicate error fixture field
- [`fabda0d`](https://github.com/yelog/lazydb/commit/fabda0df6c1c164c866fe708a043a134ffe38b19) test: update database error fixture
- [`1aad6ea`](https://github.com/yelog/lazydb/commit/1aad6eac1801325da73b9e1a34a04b78d8b01c33) merge: improve output log diagnostics and styling
- [`004b283`](https://github.com/yelog/lazydb/commit/004b28394b6a8be1485aadc4378c4493c1f279ee) docs: add output log diagnostics plan
- [`87e8ea8`](https://github.com/yelog/lazydb/commit/87e8ea8c9dc9f2906cdd58caca201762462eac89) feat(output): improve error diagnostics and styling
- [`2bbcb5f`](https://github.com/yelog/lazydb/commit/2bbcb5fbfec945450560e28be6c75debb887a727) fix(sql): complete WHERE after UPDATE assignments
- [`7bc8d18`](https://github.com/yelog/lazydb/commit/7bc8d1809eca5de060e9943701febbbacb451251) merge: add segment-aware catalog completion
- [`bbb8f2e`](https://github.com/yelog/lazydb/commit/bbb8f2e1909d12a8253025c14e3169f4c87cbf30) feat(lsp): add segment-aware catalog completion
- [`46e9a41`](https://github.com/yelog/lazydb/commit/46e9a416a637a2223a86989398a4e15d4f8e846b) merge: distinguish semantic completion icons
- [`3fbe936`](https://github.com/yelog/lazydb/commit/3fbe9366f5fe73011d525f7a743f48582431eaba) feat(completion): distinguish semantic completion icons
- [`52aa074`](https://github.com/yelog/lazydb/commit/52aa0745e4332c9efbef6a794f7d76ba1b7dfb0c) merge: complete DELETE clauses
- [`9edfc70`](https://github.com/yelog/lazydb/commit/9edfc708e7b6664d158739ac0abd95f70e3dca7a) fix(sql): complete DELETE clauses
- [`807a941`](https://github.com/yelog/lazydb/commit/807a9416d7f482c6ec2eae0d64d7fc4187e61bfe) merge: complete update target columns
- [`c9b673a`](https://github.com/yelog/lazydb/commit/c9b673ad5e8f0dc615a228b1de5d1db4659b0a93) feat(sql): complete update target columns
- [`a357db7`](https://github.com/yelog/lazydb/commit/a357db7c6e6e51cf5a8f1879fdb5abea2fb1edb4) merge: offer dialect builtin expressions in completion
- [`b5d9fd7`](https://github.com/yelog/lazydb/commit/b5d9fd7ce24a8b8b860734b88a38f48e8da92780) feat(sql): offer dialect builtin expressions in completion
- [`69bd433`](https://github.com/yelog/lazydb/commit/69bd43341b34c64aa2981940ee41a3645287b097) Merge branch 'main' into task/output-log-follow-tail
- [`c7b37ed`](https://github.com/yelog/lazydb/commit/c7b37ed0f864e31c2bc046bd62c472c09d1570da) fix(output): follow OUTPUT LOG tail on new execution
- [`a681afc`](https://github.com/yelog/lazydb/commit/a681afc477d5bf77ac357e89b0416f4550d24a21) feat(console): log target and transaction switches in OUTPUT
- [`525e540`](https://github.com/yelog/lazydb/commit/525e540bf2d7da3cb4f1e5006f1200dbaf4555ee) merge: suppress completion retrigger after accepting a candidate
- [`9b0c275`](https://github.com/yelog/lazydb/commit/9b0c27575769f80957cec2223be8d9690956ef68) fix(completion): suppress retrigger after accepting a candidate
- [`95ee5e5`](https://github.com/yelog/lazydb/commit/95ee5e5ac342ff9ff025f3b0dc82d248eab83533) merge: add safe native uninstall
- [`5d43909`](https://github.com/yelog/lazydb/commit/5d43909219a66a2c7aecc4450dd54a0d10c5d330) feat(uninstall): add safe native installation removal
- [`b58ad16`](https://github.com/yelog/lazydb/commit/b58ad16241e5f02fe342a408c4b93e8eb44cad7e) fix(ci): satisfy Rust 1.94 clippy lints
- [`23c746e`](https://github.com/yelog/lazydb/commit/23c746ee175b64bb79ba56d386f005fa9c7d8b81) merge: fix PostgreSQL row-version relation deletes
- [`e30539a`](https://github.com/yelog/lazydb/commit/e30539a287e9edc54f59b71754488152fe7b6036) fix(postgres): use row versions for relation deletes
- [`ee11bfb`](https://github.com/yelog/lazydb/commit/ee11bfb462a3981ef5cae68fcd19f99c05a31c65) fix(installer): configure shell PATH and verify installed command
- [`e4a357f`](https://github.com/yelog/lazydb/commit/e4a357f1f7a1c38dbf48bef96c120fa485796339) merge: preserve multiline SQL in execution log
- [`42e84b4`](https://github.com/yelog/lazydb/commit/42e84b42647280f6a2d34c212b464d77916e90d9) fix(output): preserve multiline SQL in execution log
- [`e8200d5`](https://github.com/yelog/lazydb/commit/e8200d5e1095d5bed8b9722195d27d2256110c68) merge: add mouse selection for text inputs
- [`f2401b0`](https://github.com/yelog/lazydb/commit/f2401b0f3a6373e2f34bb9f19d9608cdbe872d50) feat(ui): add mouse selection for text inputs
- [`97186b8`](https://github.com/yelog/lazydb/commit/97186b8fc0d6df1ca4bcd8df4c5b90c1a9155e6b) Merge branch 'task/postgres-grid-writeback'
- [`3d3eecc`](https://github.com/yelog/lazydb/commit/3d3eecc998d4d4146bf388122ef2d899bf452c3e) fix(postgres): cast typed relation mutation parameters
- [`422c5ea`](https://github.com/yelog/lazydb/commit/422c5eaa159e1d20d469ed9d850c13270519f3fc) merge: prevent notification cards from showing background
- [`c1b48b2`](https://github.com/yelog/lazydb/commit/c1b48b207d64697e4aedb9e97eacf8010e0f9ecf) fix(ui): prevent notification cards from showing background
- [`6d61424`](https://github.com/yelog/lazydb/commit/6d614240ba3e5ad0648edbd7c5beda023760a271) merge: sql editor paging and execution confirmation fix
- [`0a76f06`](https://github.com/yelog/lazydb/commit/0a76f0649bdf82de6664bed425c6d9c82cd3e5cc) fix(sql-editor): sync paging cursor and confirmation execution
- [`49ff0e8`](https://github.com/yelog/lazydb/commit/49ff0e84a3301856a48d8776c009237f77a697fb) fix(ci): install Rust lint components
- [`7650fd2`](https://github.com/yelog/lazydb/commit/7650fd2b87e5ca51eaf88fdf863722f742d1f4b6) fix(ci): honor pinned Rust toolchain
- [`e7e9b3f`](https://github.com/yelog/lazydb/commit/e7e9b3f9f759de365cede1888d5cee78563ab5f2) fix(ci): select pinned Rust toolchain
- [`d3ef7f7`](https://github.com/yelog/lazydb/commit/d3ef7f71f84c960aecd2081c330787cf82fdf413) Merge remote-tracking branch 'origin/main'
- [`0406167`](https://github.com/yelog/lazydb/commit/0406167bf8b8151bc824ade57e9abf49ae5fbb12) merge: unify notification history severity styling
- [`82e8f70`](https://github.com/yelog/lazydb/commit/82e8f70a39664d77a2a9fef68f821ac54510ee1e) feat(ui): unify notification history severity styling
- [`4ff9315`](https://github.com/yelog/lazydb/commit/4ff93155fa3844643e24442bc17fdad8c7418dd0) style(lsp): format catalog load logging
- [`326a452`](https://github.com/yelog/lazydb/commit/326a452e9b8641d591f0c97bc53a85dc7d37ea21) merge: add SQL and MyBatis language server
- [`c848f11`](https://github.com/yelog/lazydb/commit/c848f111cc9ac53a76cedd589446db94491c6ef2) feat(lsp): add SQL and MyBatis language server
- [`984b89e`](https://github.com/yelog/lazydb/commit/984b89eb667549fd11a7548d210992d97c87fea7) Update images and add alt text in README
- [`7f6d2bb`](https://github.com/yelog/lazydb/commit/7f6d2bbb0b17a9f210ccf571eaca9ff20d2db324) merge: clarify SQL editor visual selection
- [`a95d9cf`](https://github.com/yelog/lazydb/commit/a95d9cfec67856dbac44b6ba9bcec242323db2d6) fix(ui): clarify SQL editor visual selection
- [`f6e6624`](https://github.com/yelog/lazydb/commit/f6e6624cb9af3fe5b4319d26ee7b8fefa715683a) merge: add JSON cell editor cursor
- [`5b2734e`](https://github.com/yelog/lazydb/commit/5b2734e2ee726b09c548b1ea0f0887e4c514652c) fix(ui): show cursor in JSON cell editor
- [`2c24389`](https://github.com/yelog/lazydb/commit/2c243894801a7c18fecb5620b8a62c1f3e071349) fix(ui): identify default console and show notifications above modals
- [`a95c5df`](https://github.com/yelog/lazydb/commit/a95c5df16c4df55a147a214a6e2926b4903ebe7a) merge: protect the default console
- [`a2784bd`](https://github.com/yelog/lazydb/commit/a2784bd03ee952df275b17eb719526b6e3f85b2a) feat(console): protect the default console
- [`c1d3037`](https://github.com/yelog/lazydb/commit/c1d30371e8cf3afd5c9f153dbe9cb8993e3b887c) Merge branch 'task/sql-editor-height-drag'
- [`3e53de7`](https://github.com/yelog/lazydb/commit/3e53de7fde0be4e0b6d88dee10f7efd8525652c4) feat(ui): add SQL editor height dragging
- [`68f6289`](https://github.com/yelog/lazydb/commit/68f62898370e4b8d5b0fff92f431ffe7898d4dd6) merge: add typed relation cell editors
- [`d6f7ad8`](https://github.com/yelog/lazydb/commit/d6f7ad824514ae757361d7bd2cea584b43beb86e) feat: add typed relation cell editors
- [`cb8ee71`](https://github.com/yelog/lazydb/commit/cb8ee71b73349905a9f17c3f5c17a7dbafecc2d0) fix(notifications): align dismissal and history clicks
- [`bcf87a6`](https://github.com/yelog/lazydb/commit/bcf87a651d115a3a2a909d4f826793ea4bbdacb9) merge: add PostgreSQL catalog drop support
- [`1f8dc64`](https://github.com/yelog/lazydb/commit/1f8dc64ca5e1ed1a091b5faf3edd0d923f65f02e) feat(postgres): support safe catalog database and schema drops
- [`b6d6209`](https://github.com/yelog/lazydb/commit/b6d6209c77fee5909d71c42d6ec283eac6930e13) merge: add editor viewport scrolling
- [`088d7d3`](https://github.com/yelog/lazydb/commit/088d7d398971349d5378e0a66e96da3b28e876c2) fix(editor): add viewport scrolling and cursor following
- [`ecd3a77`](https://github.com/yelog/lazydb/commit/ecd3a774236d2318267bc185db5c3c79b467adb5) fix(ui): improve catalog drop confirmation dialog
- [`353d6ca`](https://github.com/yelog/lazydb/commit/353d6ca6e00b309e13fe0024285e7e1eaf0c7127) fix(postgres): decode complex values for display
- [`e064dad`](https://github.com/yelog/lazydb/commit/e064dad2a0180b9244a760f90bd93fe8cd8cfca2) merge: improve connection editor feedback and flow
- [`a2de93c`](https://github.com/yelog/lazydb/commit/a2de93c5bc79132fba422952fc561b741c6f360c) fix(ui): improve connection editor feedback and flow
- [`cf16c7d`](https://github.com/yelog/lazydb/commit/cf16c7dd9901d92998af490130ae739e0fddef74) merge: unify terminal input cursor ownership
- [`62b14eb`](https://github.com/yelog/lazydb/commit/62b14ebab50437f8fdc626049b337389c219b67e) fix(ui): unify terminal input cursor ownership
- [`2c0a10b`](https://github.com/yelog/lazydb/commit/2c0a10bc6e5f00c973d345224e79795b36ed1f90) fix(ui): enable connection group delete dialog navigation
- [`aedb624`](https://github.com/yelog/lazydb/commit/aedb6243b90b8ee4f20d4c7206baad4b246dc237) merge: fix Explorer startup selection
- [`e27d490`](https://github.com/yelog/lazydb/commit/e27d490975896308f4fa7f99c8749f9e53b895de) fix(explorer): select first visible startup row
- [`bb4c298`](https://github.com/yelog/lazydb/commit/bb4c298d85f68c3fe84acccf0ad74cda8fadd52e) fix(ui): satisfy newer clippy lints
- [`66970a2`](https://github.com/yelog/lazydb/commit/66970a23551b7efde1bc74ca1639107f1582526e) merge: add Neovim theme synchronization
- [`c5b8396`](https://github.com/yelog/lazydb/commit/c5b8396a543510c38f27fc9c98b9628c84caa41f) feat(theme): sync TUI theme from Neovim
- [`d48e2e2`](https://github.com/yelog/lazydb/commit/d48e2e2e733454ebb26e4e4fdcd86774719722f4) merge: fix SQL completion cursor and candidates
- [`fb16b87`](https://github.com/yelog/lazydb/commit/fb16b879c1d6d65c83bd52ca5f6c37d976aaeaf1) fix(sql): preserve completion cursor and candidates
- [`cc2e445`](https://github.com/yelog/lazydb/commit/cc2e445b6038afdea5af84a0ce392ee7a6b14dc3) merge: improve table editor navigation and review
- [`68e33dd`](https://github.com/yelog/lazydb/commit/68e33ddad361268e0de72c8106d57537b077cb63) feat(ui): improve table editor navigation and review
- [`6148288`](https://github.com/yelog/lazydb/commit/61482889f4c305d7f71dc5ad09501cdfe324a333) feat(ui): highlight result query clauses
- [`e1158c9`](https://github.com/yelog/lazydb/commit/e1158c929d2908220d0620bbde240c693908fedd) merge: redesign table editor discard dialog
- [`d6f0f52`](https://github.com/yelog/lazydb/commit/d6f0f5271cff90906a8fb612fc66035f703a4e98) feat(ui): redesign table editor discard dialog
- [`f84e05b`](https://github.com/yelog/lazydb/commit/f84e05b23cf23e4abc8785f04698f0250a87d98a) merge: add SQL ORDER BY completion
- [`dc99ebf`](https://github.com/yelog/lazydb/commit/dc99ebf7c23762763bf4f72c5ab269c1fe2ca3bf) feat(sql): complete ORDER BY clauses
- [`90b293a`](https://github.com/yelog/lazydb/commit/90b293a1da57d583a15a693104adf851907bc89d) merge: unify SQL highlighting
- [`62029b8`](https://github.com/yelog/lazydb/commit/62029b893993d1761a3aabb4b70b1e98c43e5c10) feat(sql): unify DDL and editor highlighting
- [`c254751`](https://github.com/yelog/lazydb/commit/c25475155e21ce65da5dba2d6c29cced159559d5) Merge catalog editor comment fixes
- [`26d01c5`](https://github.com/yelog/lazydb/commit/26d01c52d89e7bd3383308705f020d8b121a070e) fix(catalog-editor): handle table comments from column edits
- [`6042be8`](https://github.com/yelog/lazydb/commit/6042be80ef1ebafd250651d9563b30ab0a44b34a) fix(explorer): preserve styles during find
- [`3437b5c`](https://github.com/yelog/lazydb/commit/3437b5c25a23dee169f4ebe1f5690fde4b0bd9c0) merge: add explorer incremental find preview
- [`0e1bd63`](https://github.com/yelog/lazydb/commit/0e1bd63c9434ea5d1822befa0691eada2e6066a2) feat(explorer): preview and center first find match
- [`490f1fc`](https://github.com/yelog/lazydb/commit/490f1fcaa01ed4cc6874db68ea9ec5daf7e364a9) Merge remote-tracking branch 'origin/main'
- [`1abf561`](https://github.com/yelog/lazydb/commit/1abf561b79c5985d1c13ed502cd9d8fcf49d1e4a) Merge branch 'task/pagination-navigation-consistency'
- [`d034175`](https://github.com/yelog/lazydb/commit/d034175c6d9cff5f595e4f788d2ce4737fa5aaa6) feat: add consistent result pagination navigation
- [`3efe9a6`](https://github.com/yelog/lazydb/commit/3efe9a6084e4a497fe1371c4bfbf21ecb6c27351) merge: fix notification history shortcuts
- [`1aa6213`](https://github.com/yelog/lazydb/commit/1aa6213f9cd15aeb0ec6406a69b3bbf2f5b3a708) fix(notification): open history from all supported contexts
- [`74de12e`](https://github.com/yelog/lazydb/commit/74de12e96cc7c4dc05a712d169c4f6de11d5b29e) merge: fix confirmation cancel enter handling
- [`0275f7e`](https://github.com/yelog/lazydb/commit/0275f7ee195bbb749ff6b2b8aa871aaba7da5346) fix(keymap): activate focused confirmation action
- [`e27ae30`](https://github.com/yelog/lazydb/commit/e27ae30cb68d0a9df2fd02c42a6be8dcf3f1f2b3) Merge branch 'task/result-set-header-sort'
- [`e55f283`](https://github.com/yelog/lazydb/commit/e55f283228cbfad84268bb7df6d9074cb2f8ef5e) feat(ui): sort SQL result set columns
- [`abd332b`](https://github.com/yelog/lazydb/commit/abd332bdb96c4b268dd5c8ca319b446055f1cfe4) merge: complete notification center interactions
- [`ca648c8`](https://github.com/yelog/lazydb/commit/ca648c83c85c28b0393e28a0e9bc91fb6f08ce94) feat(ui): complete notification center interactions
- [`a1073dc`](https://github.com/yelog/lazydb/commit/a1073dc2c462b22dca5af0597f232da423149deb) Merge branch 'task/unified-confirmation-dialogs'
- [`ae003fb`](https://github.com/yelog/lazydb/commit/ae003fb4011054524184524d3ca8954c263e7647) feat(ui): unify confirmation dialog focus
- [`8d0c19a`](https://github.com/yelog/lazydb/commit/8d0c19a95e9399050f6a2deceac6269f888cb427) fix(ui): unify result loading status layout
- [`a2eda2c`](https://github.com/yelog/lazydb/commit/a2eda2cddba8b7e776cd2f7073a93a1411745d6d) feat(ui): use triangle sort indicators
- [`8e2ef8d`](https://github.com/yelog/lazydb/commit/8e2ef8dafe5b583a1748182199fd27f4a0384436) Merge branch 'task/mouse-copy-preview'
- [`fca9a82`](https://github.com/yelog/lazydb/commit/fca9a8226276e25950c0a9effbea287aecd3be00) feat(mouse): add transient copy preview
- [`f34ff2d`](https://github.com/yelog/lazydb/commit/f34ff2d51a38ae421fc2f795bb827b5c88e60792) Merge branch 'task/relation-header-sorting'
- [`c730563`](https://github.com/yelog/lazydb/commit/c73056349a7b5cd8266c933cee700964738075b3) feat(relation): add sortable data headers
- [`65c7f8f`](https://github.com/yelog/lazydb/commit/65c7f8f852e46457ebc813f4642148a14064bef7) Merge pull request #8 from yelog/dependabot/github_actions/actions/deploy-pages-5.0.1
- [`c83284d`](https://github.com/yelog/lazydb/commit/c83284da97bf320c05656263a9172c66e54fdde6) Merge pull request #7 from yelog/dependabot/github_actions/actions/configure-pages-6.0.0
- [`3935b7c`](https://github.com/yelog/lazydb/commit/3935b7cff26e31d547b2c5a5c06f8847d8d216fc) chore(deps): bump actions/configure-pages from 5.0.0 to 6.0.0
- [`fa11e8f`](https://github.com/yelog/lazydb/commit/fa11e8f9fd0682d54628c52b666bb21d9f26ec3e) Merge pull request #6 from yelog/dependabot/github_actions/actions/upload-pages-artifact-5.0.0
- [`d0bdd32`](https://github.com/yelog/lazydb/commit/d0bdd32b21bebd86e9cb41a7a65bed4573488b0b) merge: redesign SQL execution confirmation
- [`08fd54a`](https://github.com/yelog/lazydb/commit/08fd54ac715c11ccc9b3db0fd0f343a5f52b215b) feat(ui): redesign SQL execution confirmation
- [`ac26a04`](https://github.com/yelog/lazydb/commit/ac26a04aff0a0d0d1013af7cd76e003a020ba11a) chore(deps): bump actions/deploy-pages from 4.0.5 to 5.0.1
- [`7fc3681`](https://github.com/yelog/lazydb/commit/7fc368106ec5fbdb957655d4eba448f0ef945718) chore(deps): bump actions/upload-pages-artifact from 3.0.1 to 5.0.0
- [`c392cd1`](https://github.com/yelog/lazydb/commit/c392cd1f3183d8c7789dcf038ae03c97d91b1396) merge: add mouse selection auto-copy
- [`408c03b`](https://github.com/yelog/lazydb/commit/408c03bfce7ef89f92ccd3f8ad436ebc109d0659) feat(mouse): copy text selection on release
- [`d60fcb6`](https://github.com/yelog/lazydb/commit/d60fcb6a7eb8a32d0ced6cc20a7bd04c0166387f) merge: add explorer disclosure arrow clicks
- [`b2bdc3a`](https://github.com/yelog/lazydb/commit/b2bdc3a163f8858b4cf02aeac21bf14890bc59a9) feat(explorer): toggle nodes from disclosure arrows
- [`2599787`](https://github.com/yelog/lazydb/commit/2599787fbaff8f4feb6d694aaef36183f288fb64) merge: fix output log mouse focus
- [`3f64db3`](https://github.com/yelog/lazydb/commit/3f64db388da250d7d49d843607d57d0228bd4dca) fix(app): clear stale data query focus
- [`595a3d7`](https://github.com/yelog/lazydb/commit/595a3d7da0a58b0cee78b90dcd7ff1113dd3d810) fix(ui): focus output log on mouse click
- [`272328b`](https://github.com/yelog/lazydb/commit/272328b4f9c079c9ef2b4c49066708443fc9a21a) fix(app): allow quit while workspace has no console
- [`86907d1`](https://github.com/yelog/lazydb/commit/86907d1a97afb3ac2ac495a8911372d7df5e08cb) Merge branch 'task/mouse-cursor-placement'
- [`afba427`](https://github.com/yelog/lazydb/commit/afba427feea6dcc492eed62df5c61863c63dcdfd) feat(ui): support mouse cursor placement in text inputs
- [`25a4d23`](https://github.com/yelog/lazydb/commit/25a4d23e0a1e8a906b693da74c19daf61f80bba8) ci: stabilize pages contract ordering
- [`7453bc0`](https://github.com/yelog/lazydb/commit/7453bc086ff8b84c64f9fc0e4a69b9d94da68bde) ci: install Rust for distribution contracts
- [`a4e432c`](https://github.com/yelog/lazydb/commit/a4e432c9af3ca828497a0f31fbb39085049b894d) merge: add mouse text selection and clipboard support
- [`6589d30`](https://github.com/yelog/lazydb/commit/6589d3041815b90fa798d274146ad9e894569f44) feat(ui): add mouse text selection and clipboard support
- [`87a0ce6`](https://github.com/yelog/lazydb/commit/87a0ce6bed3bc910f36be4c74c6dff98066b6199) fix(sql-editor): keep execution target and connection aligned
- [`dab5640`](https://github.com/yelog/lazydb/commit/dab56407c6438f8b00d2789e3ed03aa3b5961946) merge: add architecture performance optimizations
- [`743ce74`](https://github.com/yelog/lazydb/commit/743ce746f3c0801f37b57ad3f51410113bfe435c) merge: add SQL editor R execution
- [`32a2fe8`](https://github.com/yelog/lazydb/commit/32a2fe8a101908d146136cf363dfb9d4a39ac44b) feat(sql-editor): run selected SQL with R
- [`fb664f1`](https://github.com/yelog/lazydb/commit/fb664f1795940b6b20dea60ce6f1fb7f28d055bd) feat(catalog): improve table editor UX
- [`c9236c4`](https://github.com/yelog/lazydb/commit/c9236c4e69e1b25ccd1d5e63b6cdfaf6fa7ec7f2) feat(catalog): improve table editor UX
- [`11a4599`](https://github.com/yelog/lazydb/commit/11a45991d1332c7aafd0a7e42db4a66bf92e7655) merge: add coding agent MCP setup
- [`8be018d`](https://github.com/yelog/lazydb/commit/8be018dff0bd31b0d378be77315453b882943a98) feat(mcp): add coding agent setup wizard
- [`354ad25`](https://github.com/yelog/lazydb/commit/354ad25e69e89432865bbb622046d45959f67379) fix(sql-editor): preserve Explorer during target switch
- [`0bbd67c`](https://github.com/yelog/lazydb/commit/0bbd67c013d47fa525fd96f0c4116b084dcfdcd1) feat: add workspace save queue and flush lifecycle
- [`7169999`](https://github.com/yelog/lazydb/commit/7169999df2839694ea1657ca0e3fc78fdc2eaf8d) fix(sql-editor): unify selector mouse interactions
- [`2dfe8a2`](https://github.com/yelog/lazydb/commit/2dfe8a2eaad6ddabd90485784671db661249ac5d) docs: design workspace save queue and failure state machine
- [`51696e1`](https://github.com/yelog/lazydb/commit/51696e129c013b8899c0d95a51e157021987bc48) perf: bound query retention and reduce workspace copy overhead
- [`3d3091f`](https://github.com/yelog/lazydb/commit/3d3091fd56237bb7a83563b826f78b698942dffa) merge: integrate SQL statement indicator
- [`8e1a242`](https://github.com/yelog/lazydb/commit/8e1a242f3e193fa41732d6f2b08fd4496b7b89f7) fix(ui): replace SQL statement underlines with gutter marker
- [`4cbb56b`](https://github.com/yelog/lazydb/commit/4cbb56b967ea148d5619112d5483c25465f09be6) merge: add mouse pane resizing
- [`ae11660`](https://github.com/yelog/lazydb/commit/ae11660f59d7f33b3426eed9667b20b52e6eed85) feat(ui): resize explorer pane with mouse drag
- [`9fa6158`](https://github.com/yelog/lazydb/commit/9fa6158ee46a917b4ed0f44ce5d63412d8c013b6) merge: support PostgreSQL inserts without primary keys
- [`d707ccc`](https://github.com/yelog/lazydb/commit/d707cccc76db2ede53d2d3b9cab8ff9851206123) fix(postgres): allow inserts without primary keys
- [`8b50786`](https://github.com/yelog/lazydb/commit/8b50786e458e41f59a133057f15b3e23096662a5) merge: add SQL editor mouse controls
- [`d2786b3`](https://github.com/yelog/lazydb/commit/d2786b3e6277e51c38a14539d6ba75880a691348) feat(sql-editor): add mouse controls for target and transactions
- [`51c6f3f`](https://github.com/yelog/lazydb/commit/51c6f3f2b759e3ea93a47b351c047eb1c8d78204) merge: fix SQL completion trigger behavior
- [`fb3993d`](https://github.com/yelog/lazydb/commit/fb3993d6b6f3d81f20a7708b088acf97c289cc34) fix(sql): gate automatic completion on active input
- [`aa15b16`](https://github.com/yelog/lazydb/commit/aa15b1681136bb6b51a45aeb518f8a4c61e461af) ci(release): gate distribution and verify online installation
- [`a783bd6`](https://github.com/yelog/lazydb/commit/a783bd6a2383451af79049dba97f2bcd7a408af7) fix(installer): align manifest targets with release assets
- [`31020c1`](https://github.com/yelog/lazydb/commit/31020c1321adaadf93920ab0820777ecc540463e) fix(input): keep focus in explorer for empty workspaces

## [0.1.2] - 2026-09-09

### Changed

- Improved native update detection, archive compatibility, and installer behavior across supported platforms ([`7b631eb`](https://github.com/yelog/lazydb/commit/7b631eb722ec1a6a3259991a1a554ea12ba5fc75), [`0b937ad`](https://github.com/yelog/lazydb/commit/0b937ad6b973b25122fe326aa1210b8d1ce0b48c), [`98a8be9`](https://github.com/yelog/lazydb/commit/98a8be97e572443bba571b97f6d5e3a96735c307)).
- Made macOS release binaries independent of Homebrew `xz` and strengthened distribution packaging and recovery checks ([`c949da8`](https://github.com/yelog/lazydb/commit/c949da844e0225567c04fe0ac8f8c1489cb3b60a), [`eef0c78`](https://github.com/yelog/lazydb/commit/eef0c78bbfc331545fa14783c49062d54ce55b2c)).

### Internal

- Reduced CI disk usage and removed Cargo build artifact caching ([`fea9da7`](https://github.com/yelog/lazydb/commit/fea9da75105d1fb006237967bf97cb7bd3db9d29), [`fb2d5b3`](https://github.com/yelog/lazydb/commit/fb2d5b322f2a77ecf0f8171e9d466612c1f96ecb)).
- Updated release automation, documentation, and integration coverage for the new distribution flow ([`c676606`](https://github.com/yelog/lazydb/commit/c6766063391901bbb689f65b3ada346c6743474e), [`7ac3c9c`](https://github.com/yelog/lazydb/commit/7ac3c9c39fd30eb50052ea6e16418c378ec4893e)).
- Added deterministic version rollover policy checks to release tooling ([`fd6d1a7`](https://github.com/yelog/lazydb/commit/fd6d1a71afa18b29960e9263a0579e1f7004b6c8)).

### Commits

- [`fd6d1a7`](https://github.com/yelog/lazydb/commit/fd6d1a71afa18b29960e9263a0579e1f7004b6c8) fix(release): use deterministic version rollover policy
- [`c676606`](https://github.com/yelog/lazydb/commit/c6766063391901bbb689f65b3ada346c6743474e) chore(release): automate publishing after version confirmation
- [`eef0c78`](https://github.com/yelog/lazydb/commit/eef0c78bbfc331545fa14783c49062d54ce55b2c) fix(release): unblock tagged release recovery
- [`7ac3c9c`](https://github.com/yelog/lazydb/commit/7ac3c9c39fd30eb50052ea6e16418c378ec4893e) Update MCP and Neovim LSP headings in README
- [`98a8be9`](https://github.com/yelog/lazydb/commit/98a8be97e572443bba571b97f6d5e3a96735c307) fix(installer): simplify Windows bootstrap with text endpoint
- [`be6a339`](https://github.com/yelog/lazydb/commit/be6a339c7252f372b89798610f4e39a069a7456d) Merge remote-tracking branch 'origin/main'
- [`0b937ad`](https://github.com/yelog/lazydb/commit/0b937ad6b973b25122fe326aa1210b8d1ce0b48c) fix(update): accept archive directory markers and preserve legacy compatibility
- [`c949da8`](https://github.com/yelog/lazydb/commit/c949da844e0225567c04fe0ac8f8c1489cb3b60a) fix(release): make macOS binaries independent of Homebrew xz
- [`fea9da7`](https://github.com/yelog/lazydb/commit/fea9da75105d1fb006237967bf97cb7bd3db9d29) ci: reduce cargo test disk usage
- [`fb2d5b3`](https://github.com/yelog/lazydb/commit/fb2d5b322f2a77ecf0f8171e9d466612c1f96ecb) ci: avoid caching cargo build artifacts
- [`7b631eb`](https://github.com/yelog/lazydb/commit/7b631eb722ec1a6a3259991a1a554ea12ba5fc75) fix(update): recognize resolved native launchers and report check errors

## [0.1.3] - 2026-09-09

### Changed

- Improved workspace save recovery when quitting, including empty-workspace handling and persistence coverage ([`fed1ce0`](https://github.com/yelog/lazydb/commit/fed1ce0e22d237cb11933ba1e82bea731e18ea2c)).
- Added discovery and reuse of existing MCP client configurations, with setup and doctor support ([`cd65a3e`](https://github.com/yelog/lazydb/commit/cd65a3ed287b17d5586e9c0ab4d87be14cd70929)).

### Internal

- Updated CI compatibility for the latest Clippy lints ([`ba33cf2`](https://github.com/yelog/lazydb/commit/ba33cf261f039cfeae11e9aa25b856e09561c1bd)).
- Simplified installation documentation ([`4c14e85`](https://github.com/yelog/lazydb/commit/4c14e8519c69022b789b4f215f689c6db433247f)).
- Cleared committed relation edits before refresh and expanded relation tab coverage ([`bf41f2e`](https://github.com/yelog/lazydb/commit/bf41f2e49de532e70135f65227a506f62a8451c4)).

### Commits

- [`9023973`](https://github.com/yelog/lazydb/commit/90239734589c686a8b57615af7fb4e8dd1c9c673) merge: recover from failed quit saves
- [`fed1ce0`](https://github.com/yelog/lazydb/commit/fed1ce0e22d237cb11933ba1e82bea731e18ea2c) fix(workspace): recover from failed quit saves
- [`ba33cf2`](https://github.com/yelog/lazydb/commit/ba33cf261f039cfeae11e9aa25b856e09561c1bd) fix(ci): satisfy latest clippy lints
- [`cd65a3e`](https://github.com/yelog/lazydb/commit/cd65a3ed287b17d5586e9c0ab4d87be14cd70929) fix(mcp): discover and reuse existing client configurations
- [`4c14e85`](https://github.com/yelog/lazydb/commit/4c14e8519c69022b789b4f215f689c6db433247f) docs(readme): simplify installation instructions
- [`bf41f2e`](https://github.com/yelog/lazydb/commit/bf41f2e49de532e70135f65227a506f62a8451c4) fix(relation): clear committed edits before refresh

## [0.1.4] - 2026-09-11

### Added

- Added SQL editor diagnostics, jump-list navigation, workspace database selection, unified pane scrollbar interactions, draggable result-grid scrolling, and improved SQL editor indentation and selection.
- Added home-directory installation support and post-confirmation MCP setup for installers.

### Changed

- Improved catalog relation reads with request-scoped visibility and streamlined database selector interactions.
- Improved installer application-data-root handling and SQL editor shortcut help.

### Fixed

- Corrected diagnostic rendering ranges and platform-specific Clippy compatibility.

### Internal

- Updated release build parallelization, artifact caching, validation timeouts, and editor diagnostic implementation documentation.

### Commits

- [`b8142fe`](https://github.com/yelog/lazydb/commit/b8142fe7528859f872700934f0d68a1ca36fa605) fix(ci): satisfy platform-specific clippy
- [`9f41238`](https://github.com/yelog/lazydb/commit/9f412384917a12131786bc7fd9db4eefe6b8876a) ci(release): parallelize builds and cache Rust artifacts
- [`42c2fdb`](https://github.com/yelog/lazydb/commit/42c2fdbbc36fefc3a92eec7fc14cf453823f58bb) fix(installer): unify application data root
- [`a86dce3`](https://github.com/yelog/lazydb/commit/a86dce30427c2173cb3fd1f3e3c7c275d7b9f6c0) docs: add editor diagnostic rendering plan
- [`6331548`](https://github.com/yelog/lazydb/commit/6331548e73f54396d28ad811f6f7d99fce7b0f97) fix(help): show SQL editor shortcut
- [`732a412`](https://github.com/yelog/lazydb/commit/732a41268d908e69a550bafad6e733989643633d) Merge branch 'task/sql-diagnostic-rendering'
- [`34b9423`](https://github.com/yelog/lazydb/commit/34b94230ed1349b8013bc5602ebecfb83f833459) fix(sql): align diagnostic rendering ranges
- [`5222dbc`](https://github.com/yelog/lazydb/commit/5222dbc9e0a30690a093f043be15f58d6bf2f8cb) merge: streamline database selector
- [`80ed748`](https://github.com/yelog/lazydb/commit/80ed748ba70d109634a51892490c0cd15f3e28aa) fix(ui): streamline database selector
- [`5c55ad7`](https://github.com/yelog/lazydb/commit/5c55ad7c6908e1f522d978af0c2b001ce2cbfadc) merge: add SQL editor jump list
- [`81f6a93`](https://github.com/yelog/lazydb/commit/81f6a93b1ed0e04be76f10569059ecea9f97e208) feat(sql-editor): add jump list navigation
- [`9ba151d`](https://github.com/yelog/lazydb/commit/9ba151df2bc72b61e91211a56c2126de314cb1b7) merge: add SQL editor diagnostics
- [`ceee037`](https://github.com/yelog/lazydb/commit/ceee03786f77e70ca0035dd88248d5e075734f33) feat(sql): add editor diagnostics
- [`e2b48a6`](https://github.com/yelog/lazydb/commit/e2b48a6d8011a9576fb2a6ea31f85a6b4fc17d57) merge: add workspace database selector
- [`287eb82`](https://github.com/yelog/lazydb/commit/287eb82078e1676080d395cddebf3bfa0c0f1036) feat(ui): add workspace database selector
- [`9ccbfd6`](https://github.com/yelog/lazydb/commit/9ccbfd6f5ea03255dd38caf618621d470b5603ab) Merge branch 'task/unified-pane-scrollbars'
- [`2f9da5b`](https://github.com/yelog/lazydb/commit/2f9da5bd08545fbda51651154cf31e6d3e12d325) feat(ui): unify pane scrollbar interactions
- [`ef603e8`](https://github.com/yelog/lazydb/commit/ef603e8bbc53f08479ed0ae7761b7ed3b91a148c) merge: fix request-scoped visible objects
- [`66557a2`](https://github.com/yelog/lazydb/commit/66557a24c211dc3627fff19cd8d4830ee24ef3d0) fix(catalog): use request scope for relation reads
- [`c9afca5`](https://github.com/yelog/lazydb/commit/c9afca5d095e63826d66f036d8539605849d1fff) merge: integrate SQL editor indentation and selection fixes
- [`8c731a2`](https://github.com/yelog/lazydb/commit/8c731a2501ec2dd5e104841774d2e53536cbf724) fix(sql-editor): preserve selections and support indentation
- [`1ec9e46`](https://github.com/yelog/lazydb/commit/1ec9e46663c605c22b7415d5b44688bc6922e692) feat(grid): add draggable vertical table scrollbar
- [`ae98291`](https://github.com/yelog/lazydb/commit/ae98291e61020d7a369d1f070d00f7ea162842a9) merge: support home directory installations
- [`9b2e037`](https://github.com/yelog/lazydb/commit/9b2e037a6c83855183e743b3e0cd7929afa15f47) feat(paths): support home directory installations
- [`e98abbd`](https://github.com/yelog/lazydb/commit/e98abbd927e36e7d4de99e75e39387f47b7058bb) fix(installer): run MCP setup after confirmation
- [`ae95a6e`](https://github.com/yelog/lazydb/commit/ae95a6e12f7ab99d8d84c573a2561020a7bb8a1f) ci(release): allow longer validation timeouts

## [0.1.5] - 2026-09-16

### Added

- Added Redis database browsing, typed value previews, preview formatting and navigation, SQL execution history, concurrent database consoles, Oracle and MariaDB capabilities, OpenCode/MCP compatibility, and multi-database catalog mutations.

### Changed

- Improved workspace persistence, connection/session routing, Redis scanning and preview performance, SQL history search and presentation, Omni navigation, catalog forms, database branding, and restored-tab lifecycle behavior.

### Fixed

- Fixed connection recovery, metadata isolation, catalog loading, generated-column handling, transaction and history persistence, UI focus/navigation, asynchronous test behavior, and cross-database integration edge cases.

### Internal

- Expanded release documentation, implementation plans, fixtures, CI coverage, lint compatibility, and regression tests across the supported database drivers and terminal UI.

### Commits
- [`59cf4ea`](https://github.com/yelog/lazydb/commit/59cf4ea2d913e6f1ac79b9c6d07d16b580b8ed43) Merge branch 'task/sql-editor-target-selector-icon-order'
- [`027f782`](https://github.com/yelog/lazydb/commit/027f7824549abec4bcb88fdd8a3c216ee43edf93) docs: add target selector icon order plan
- [`8f53242`](https://github.com/yelog/lazydb/commit/8f5324285a3c2bc78e70d210d1352509e0ee8a50) merge: sql editor target selector icon order
- [`075b59d`](https://github.com/yelog/lazydb/commit/075b59db591f0c1d72ca52aed9876db9a55263a3) fix(ui): move database icon before connection name
- [`f1d8974`](https://github.com/yelog/lazydb/commit/f1d897400be6cb356194d2bfe3d4b11eee15e989) merge: target selector visual polish
- [`3bdbc35`](https://github.com/yelog/lazydb/commit/3bdbc3531d3166ea7495a2b116d2c71b1714d6c6) fix(ui): polish target selector
- [`557a808`](https://github.com/yelog/lazydb/commit/557a8088607945ec5745809c703fd12912a42c17) Merge branch 'task/lazy-console-focus-connect'
- [`e9de0c7`](https://github.com/yelog/lazydb/commit/e9de0c79f64564e9a07574ec93ce8da6932fddb5) docs: add lazy console focus plan
- [`4b956d7`](https://github.com/yelog/lazydb/commit/4b956d76698dc206b1fd4bffce17b42cb560502f) merge: lazy console focus connection
- [`39079a5`](https://github.com/yelog/lazydb/commit/39079a59504a8ab3583d768d7f99abd69b2d32a5) fix(console): connect restored target on focus
- [`5ea15a7`](https://github.com/yelog/lazydb/commit/5ea15a79c351d50e79c4422dad9133d8e4fea8d2) fix(sql-editor): restrict targets to relational databases
- [`6fc85a9`](https://github.com/yelog/lazydb/commit/6fc85a946e0c291abb2d02d71fe710bd97768331) docs(plans): add implementation plans
- [`794196e`](https://github.com/yelog/lazydb/commit/794196e2f0ca1284bed2b1df28c2a89406d976fc) Merge branch 'task/redis-key-value-focus-navigation'
- [`5c97331`](https://github.com/yelog/lazydb/commit/5c97331add117b3fe214f8b036611960ee0ae664) docs(redis): add focus navigation plan
- [`2bb57a9`](https://github.com/yelog/lazydb/commit/2bb57a971fdedde9151b851a666853136fa1a8cc) merge: redis key value focus navigation
- [`5339639`](https://github.com/yelog/lazydb/commit/5339639d200576c0a8db2889c1fc4738783b9711) fix(redis): navigate key and value focus panes
- [`6e46990`](https://github.com/yelog/lazydb/commit/6e469907120147e9ac21bc265a69642d1d74dcf4) Merge branch 'task/redis-keys-pane-resize'
- [`eba2c2a`](https://github.com/yelog/lazydb/commit/eba2c2a06bfcac85dbf5dc6fa0a9b9c272f6fb85) docs(redis): add keys pane resize plan
- [`490ecac`](https://github.com/yelog/lazydb/commit/490ecac0aa7e4c4d5f827eef8d5ac582c9b3fc0e) merge: redis keys pane resize
- [`cba5e09`](https://github.com/yelog/lazydb/commit/cba5e096690e663d11095f6f632605545ad1e45a) feat(redis): resize keys pane
- [`8857f18`](https://github.com/yelog/lazydb/commit/8857f185ae4457a86a7c7148be6df3658a250001) Merge branch 'task/redis-keys-group-toggle-counts'
- [`0818e8d`](https://github.com/yelog/lazydb/commit/0818e8dd6c8d2271b0d01af5178d3692abe67527) docs(redis): add group toggle counts implementation plan
- [`c2eaa1c`](https://github.com/yelog/lazydb/commit/c2eaa1c6da690500fb63aaa00a5a41e71614f8e3) merge: redis keys group toggle and counts
- [`fbb6235`](https://github.com/yelog/lazydb/commit/fbb62352bf1e9eaa1b94882e157dc82c054388cf) fix(redis): restore group toggle shortcut and simplify counts
- [`56e1ded`](https://github.com/yelog/lazydb/commit/56e1ded88a083b4193f1c330d4609c8a7154e65f) Merge branch 'task/redis-context-help'
- [`ddfd946`](https://github.com/yelog/lazydb/commit/ddfd946d0954357e490e3406260736e7f0855f62) fix(redis): add contextual shortcut help
- [`ba8573f`](https://github.com/yelog/lazydb/commit/ba8573f688f5828c3c6d231899d94c2d285cf5d6) merge: redis key tree interaction improvements
- [`fa82a66`](https://github.com/yelog/lazydb/commit/fa82a66cbb72cb117946826cc8e96096c9f8cc12) feat(redis): refine key tree navigation and preview
- [`1cef150`](https://github.com/yelog/lazydb/commit/1cef150c48ad4cf62fe5d02475b794c97ac553cb) merge: console auto-connect workspace improvements
- [`b62f228`](https://github.com/yelog/lazydb/commit/b62f2280b3db6da9d97dc8b14e8f87f04b00d558) test(console): align fixtures with target sessions
- [`7449be4`](https://github.com/yelog/lazydb/commit/7449be47553f29547651b6e5ad6bc54527927942) fix(console): show target session status and retry failures
- [`5db289a`](https://github.com/yelog/lazydb/commit/5db289acefb1071a8fb95e8409f282699c61dc52) fix(console): preserve latest activation during connection changes
- [`871d516`](https://github.com/yelog/lazydb/commit/871d5167ff6de8c2c8ce19239af3c22bcbb56a99) fix(startup): connect the restored active console target
- [`8fb78ef`](https://github.com/yelog/lazydb/commit/8fb78eff85b0c2a318fd6007c351b0b330220357) fix(console): prepare target connection on creation
- [`080e0c8`](https://github.com/yelog/lazydb/commit/080e0c8af96fca1c59dc6656c01f9a02bae44031) fix(ui): keep offline console workspace visible
- [`2afc77e`](https://github.com/yelog/lazydb/commit/2afc77e1ec273bfab9fd61095d79455240ab942d) fix(workspace): silence successful autosave notifications
- [`162cdca`](https://github.com/yelog/lazydb/commit/162cdca80d9311df9b3201db6d327a005d7dee41) Merge branch 'task/console-manager-empty-startup'
- [`c4e8d19`](https://github.com/yelog/lazydb/commit/c4e8d190c0490b0c8780f76f56672921b76a7588) fix(console): allow management without active console
- [`a76f1da`](https://github.com/yelog/lazydb/commit/a76f1da4c4b3d59df411852d8dc389b61e91ecca) perf(completion): remove global index projection
- [`32147ae`](https://github.com/yelog/lazydb/commit/32147ae144cb1f4165f999aeb4d74662778fd454) fix(sql): filter fallback relations by namespace
- [`b685be0`](https://github.com/yelog/lazydb/commit/b685be0df7fda06410c25e46a7d35803fbe2eab2) fix(completion): ignore unrelated catalog refreshes
- [`5328de7`](https://github.com/yelog/lazydb/commit/5328de78931c0e7bef6fcd63d0c9f2427328cde7) fix(completion): bind requests to editor targets
- [`8f9bd25`](https://github.com/yelog/lazydb/commit/8f9bd25e63205e97c50cb5e6df9fea083b277f05) fix(completion): isolate target catalog metadata
- [`ddc6222`](https://github.com/yelog/lazydb/commit/ddc62226c8695e66311685c277599bf57a73f06b) merge: add SQL history type tags
- [`8107bd1`](https://github.com/yelog/lazydb/commit/8107bd199e80f31c1652eeffb4a2e6ab4bc013bb) feat(sql-history): add statement type tags
- [`b237028`](https://github.com/yelog/lazydb/commit/b237028feb6a84b9605616deebded87f83538355) Merge branch 'task/redis-table-data-grid'
- [`e0db21e`](https://github.com/yelog/lazydb/commit/e0db21e30f3b0f3237beba573f2d10d1bcaedca7) merge: persist offline tab closures
- [`3007140`](https://github.com/yelog/lazydb/commit/30071408490aa4b15d830a493a60131ee13d30be) fix(workspace): persist offline tab closures
- [`4556f79`](https://github.com/yelog/lazydb/commit/4556f79b5b80988b31b8879ee74535dd8d3c7e0e) fix(workspace): persist offline tab closures
- [`97b26e1`](https://github.com/yelog/lazydb/commit/97b26e139b40c583bc420bb976a174a9b35cf6f6) fix(redis): satisfy table keymap lint
- [`9d7f300`](https://github.com/yelog/lazydb/commit/9d7f30092e691715b4d63ec8476f82046aaa364a) feat(redis): show table loading status
- [`4d2f090`](https://github.com/yelog/lazydb/commit/4d2f0906da8bf85ad064f66d6094d7b57269c75c) feat(redis): merge overlapping value pages
- [`7a14738`](https://github.com/yelog/lazydb/commit/7a14738d5fa16a71516dfce7ec9c31c9c94f39d1) feat(redis): guard incremental value loading
- [`34f2004`](https://github.com/yelog/lazydb/commit/34f2004ddccb509e54079a7e537a1767323d0462) feat(redis): copy raw table cells
- [`de69e09`](https://github.com/yelog/lazydb/commit/de69e09b03291aea61a7fd4118a2ecd3c5474ac9) feat(redis): route table input to grid
- [`6ca1541`](https://github.com/yelog/lazydb/commit/6ca1541ebd1f3fc0af5d91516ac8a5d5e3f51437) feat(redis): render tables through shared data grid
- [`ef86643`](https://github.com/yelog/lazydb/commit/ef866436c32dc17756c3c37d9f822949e0efd172) feat(redis): add preview grid state
- [`374d5c5`](https://github.com/yelog/lazydb/commit/374d5c51823339461a29155ce6e998b67c054cf3) feat(redis): preserve table row identities
- [`3aab539`](https://github.com/yelog/lazydb/commit/3aab539f6edfbea054490761cdfbbeb387f03cdb) test(redis): establish table preview contract
- [`3cf4d5b`](https://github.com/yelog/lazydb/commit/3cf4d5bee17db02ec4ffc590f18934695fe0f951) docs: add Redis lifecycle implementation plans
- [`ca295a0`](https://github.com/yelog/lazydb/commit/ca295a00635cccfe07b54b12186dbb2c8868b0b9) merge: global offline console lifecycle
- [`949549b`](https://github.com/yelog/lazydb/commit/949549b9c86d23d80ac0fc9a4d9082d7f2d83135) feat(console): support global offline console lifecycle
- [`c3a9c62`](https://github.com/yelog/lazydb/commit/c3a9c622cc27fe8b24b9970d351cbcd634c22a03) merge: redis browser loading lifecycle
- [`e7cce6e`](https://github.com/yelog/lazydb/commit/e7cce6ee0c0746319673554934326346e9514c24) fix(ui): show Redis browser loading status
- [`5f2b955`](https://github.com/yelog/lazydb/commit/5f2b955eb34a919eda7253533c2c6adb3739753e) perf(redis): bound initial empty scan continuation
- [`dac961f`](https://github.com/yelog/lazydb/commit/dac961f828634bede17063264de85cf4f1945d99) fix(redis): load restored browser tabs
- [`12969f6`](https://github.com/yelog/lazydb/commit/12969f6a383ad5074d06638757e2e323935ddfac) fix(explorer): style Redis database nodes
- [`eefd04d`](https://github.com/yelog/lazydb/commit/eefd04d77af1198957fafcc719df3cfc3ee0442a) merge: oracle catalog drop support
- [`627f105`](https://github.com/yelog/lazydb/commit/627f105bcecdc159625fb6c5da4bd34a90792fac) feat(oracle): support catalog object drops
- [`c3be470`](https://github.com/yelog/lazydb/commit/c3be4706c6df6cb00b68a49a1d06264308861cde) merge: integrate consoles panel layout
- [`3fdc86d`](https://github.com/yelog/lazydb/commit/3fdc86d9fa9d797c1b0b40bc439180db349bc9e8) feat(ui): simplify consoles status and align metadata
- [`99c24ae`](https://github.com/yelog/lazydb/commit/99c24aea4a493ce5e78de37b0de34ccad3a488ce) fix(output): style all log timestamps as muted
- [`03106c3`](https://github.com/yelog/lazydb/commit/03106c3047ad08451368f6b0f7bf7e71981d3bfc) ci: fix MariaDB step indentation
- [`09fb2b1`](https://github.com/yelog/lazydb/commit/09fb2b143046ebde03fb0d7037e754173769b29f) ci: keep MySQL tests out of MariaDB job
- [`edcd5fb`](https://github.com/yelog/lazydb/commit/edcd5fb88de3afb46b340fabe82215ff30641a98) test(mariadb): tolerate empty result metadata
- [`f868003`](https://github.com/yelog/lazydb/commit/f868003e161a2d56ce942d909764971c7f670f16) fix(ci): handle catalog and MariaDB test fixtures
- [`2df2e55`](https://github.com/yelog/lazydb/commit/2df2e551c68c8830702344d242d3c484034f6c5b) fix(ci): stabilize cross-platform integration tests
- [`795f35d`](https://github.com/yelog/lazydb/commit/795f35d84a3c35d8214c211469ac1407c17a70a0) fix(ci): satisfy Rust 1.94 test checks
- [`bdd0539`](https://github.com/yelog/lazydb/commit/bdd05392f17a828ea8f9e8964da9ce8f1a84c371) fix(ui): improve selected cell contrast
- [`8c05a1c`](https://github.com/yelog/lazydb/commit/8c05a1ce7e952cced7f2bfda818fdaabeb73b96d) docs: add implementation plans
- [`bdaa09e`](https://github.com/yelog/lazydb/commit/bdaa09ef861fa23bb2962daf5eff306c44cdd9a0) fix(ui): align data grid selection
- [`2687d51`](https://github.com/yelog/lazydb/commit/2687d511ce20f6fb9b51b35f27402fe0698fe672) fix(explorer): preserve catalog session during relation restore
- [`1ba0680`](https://github.com/yelog/lazydb/commit/1ba068045dae6d9d8df1bd18ab5f06232688d576) Merge branch 'task/redis-preview-navigation-performance'
- [`97bb9e2`](https://github.com/yelog/lazydb/commit/97bb9e213a8e7909f79be7f626d7d61634ead2f3) perf(redis): optimize preview navigation
- [`7b1ecaf`](https://github.com/yelog/lazydb/commit/7b1ecaf3e7c8ef92bdee8ea41fbfe5631ac0b167) Merge branch 'task/explorer-connection-loading-lifecycle'
- [`24c5f49`](https://github.com/yelog/lazydb/commit/24c5f4984e002dde9062234f0b370145529b62f2) fix(explorer): isolate concurrent catalog loading
- [`2089397`](https://github.com/yelog/lazydb/commit/20893976eeba69cdc33fd9a255c93293708a5c92) fix(ui): avoid panic while preparing relation connections
- [`da2ffef`](https://github.com/yelog/lazydb/commit/da2ffef36a3b0cd36613d25eb28a59b2ce931159) merge: clarify redis explorer database presentation
- [`2a74b65`](https://github.com/yelog/lazydb/commit/2a74b65515e12ecc15c19be4c91495578b386035) fix(redis): clarify database explorer and tab labels
- [`4ca575d`](https://github.com/yelog/lazydb/commit/4ca575d37c92123a5698f44121e13ef4ee7b69ad) fix(relation): avoid preloading inactive restored tabs
- [`467f8e5`](https://github.com/yelog/lazydb/commit/467f8e5476f3a42c03296d05e13632f62a8dc279) fix(ui): distinguish relation preparation from empty data
- [`3fc08ab`](https://github.com/yelog/lazydb/commit/3fc08ab06d65d8263a4c5879e6d6b369d4847b61) fix(relation): resume previews after target connection
- [`6e4e766`](https://github.com/yelog/lazydb/commit/6e4e766f81b1ef4153e988a65a42c71e32349861) fix(relation): decouple previews from catalog readiness
- [`d2f835d`](https://github.com/yelog/lazydb/commit/d2f835dd2ad07f13e8f5f3677f7157aa7c49c0c7) feat(relation): track preview preparation state
- [`da6bda6`](https://github.com/yelog/lazydb/commit/da6bda6aaecd6c56b85baa7aef4b2b0359448095) feat(relation): track preview preparation state
- [`b75173f`](https://github.com/yelog/lazydb/commit/b75173f8b2c1cc0bf5c4b3e6f969153f1b919505) fix(relation): prepare target sessions for restored previews
- [`3e3873d`](https://github.com/yelog/lazydb/commit/3e3873daaf4b833e212d3f6cd571145ba1fff0ed) merge: unify console workspace persistence
- [`6257c8c`](https://github.com/yelog/lazydb/commit/6257c8c7af48399e2897f2fea33762f4295ef657) fix(workspace): unify console persistence identity
- [`0d02ff0`](https://github.com/yelog/lazydb/commit/0d02ff0596fdc3651d4d71267534957c916f3354) merge: oracle table datatype validation
- [`4011f80`](https://github.com/yelog/lazydb/commit/4011f809ec0e73921b98a5b1aa908086bae2cadb) fix(oracle): prevent invalid table datatypes
- [`2af933f`](https://github.com/yelog/lazydb/commit/2af933ff8c57d21bd97c74b134f1f94851e44d52) fix(redis): restore preview session with current format
- [`2bd662c`](https://github.com/yelog/lazydb/commit/2bd662ccd59192f855f7ca390f0e7fe39d91834d) merge: add Redis Preview Vim navigation
- [`da159a7`](https://github.com/yelog/lazydb/commit/da159a7d508f58de1b5ca28a3e20d561b5bf838a) Merge branch 'task/redis-serialization-preview'
- [`82677c1`](https://github.com/yelog/lazydb/commit/82677c15b87b7d3dd56ef46f9e20683572416950) feat(redis): preview serialized values automatically
- [`0fb90a0`](https://github.com/yelog/lazydb/commit/0fb90a0642d681041271cee70ad65a60cc28293b) merge: redesign database and role forms
- [`eb902f6`](https://github.com/yelog/lazydb/commit/eb902f64b3acb3bc3fb645f743acca94032df4fc) feat(ui): align database and role forms with connection design
- [`b4eaca6`](https://github.com/yelog/lazydb/commit/b4eaca6691bdb61fc517f5da4906b2e848126051) chore: satisfy strict clippy checks
- [`80c49bf`](https://github.com/yelog/lazydb/commit/80c49bfb191c97ef4c14ee5d2ac9d1e1065810cb) feat(redis): show preview vim mode and contextual help
- [`46ce7c8`](https://github.com/yelog/lazydb/commit/46ce7c809f448b446de9c5c6c1f5776deefae20f) merge: MariaDB support improvements
- [`0c09da3`](https://github.com/yelog/lazydb/commit/0c09da36ab1fd1c60b4fa90512cf7b2c5eabaf27) docs(mariadb): publish verified support matrix
- [`7236733`](https://github.com/yelog/lazydb/commit/7236733c55a7d7b42a3eddf79445953ac53f5a15) fix(editor): align wrapped preview paging with cursor state
- [`2b7336e`](https://github.com/yelog/lazydb/commit/2b7336e635b00498c768b3479d75297f919d1e6f) feat(redis): add conflict-free preview shortcuts
- [`61c093d`](https://github.com/yelog/lazydb/commit/61c093d81e01a1ba695280f677d7efc2ee111502) perf(mariadb): bound metadata and monitoring work
- [`5164498`](https://github.com/yelog/lazydb/commit/5164498f6252d216435caf65d2a96fe5460d8015) test(mariadb): certify supported connection options
- [`34ade27`](https://github.com/yelog/lazydb/commit/34ade27f3d7c2d6fca17a474c90ee518406e230d) fix(ui): satisfy clippy for grid selection helper
- [`653daca`](https://github.com/yelog/lazydb/commit/653daca55247f647b656781bbff2e7a19033ea2c) fix(redis): route preview keys through read-only editor
- [`bb6917a`](https://github.com/yelog/lazydb/commit/bb6917a32ed24eebf3033f28183aba605df5ba93) test(mariadb): preserve system-versioned table semantics
- [`3b8353e`](https://github.com/yelog/lazydb/commit/3b8353ec59adbe1a7c7e08f901354fb2b2708f76) test(mariadb): keep unimplemented principal mutations gated
- [`acb72a4`](https://github.com/yelog/lazydb/commit/acb72a4716c10639198428d75e287d6ccf38557f) test(mariadb): verify trigger definition round-trip
- [`5421cfa`](https://github.com/yelog/lazydb/commit/5421cfaba156e2b64a3b59095a15170b86266f44) fix(sql): preserve MariaDB client delimiter boundaries
- [`94d26b7`](https://github.com/yelog/lazydb/commit/94d26b7d4fcc83d5a36c5ac360c720ca8454b572) merge: oracle DDL execution fix
- [`22a80f1`](https://github.com/yelog/lazydb/commit/22a80f1f4fe8f4d5cbd390ac1f02981491db527d) fix(oracle): allow ordinary DDL execution
- [`ba2346c`](https://github.com/yelog/lazydb/commit/ba2346cab39b40c844641dca319757b954f6b50e) docs(plans): add Redis preview implementation plan
- [`37671ed`](https://github.com/yelog/lazydb/commit/37671edc0bc9fd2a6b99ee081796efcc0b075fc5) feat(sql): add MariaDB dialect context
- [`479a897`](https://github.com/yelog/lazydb/commit/479a8970fff526c72887e92b5e4afb438a983246) feat(mariadb): add guarded sequence drop planning
- [`143d198`](https://github.com/yelog/lazydb/commit/143d19877c4174efc1fe3baa579b12460fa69c3a) fix(ui): preserve result colors during transitions
- [`678eb9c`](https://github.com/yelog/lazydb/commit/678eb9cfce5ff59f191a7f11cb2a7d7f8271d205) feat(mariadb): expose sequence catalog and search
- [`636e3bd`](https://github.com/yelog/lazydb/commit/636e3bde57a02613eb9a32ae81a1112ddde0599d) feat(mariadb): expose structured check constraints
- [`96f5feb`](https://github.com/yelog/lazydb/commit/96f5feba6fce8e705930eeeed0523149cf6a6305) test(mariadb): certify table and view definition round-trips
- [`4b33979`](https://github.com/yelog/lazydb/commit/4b3397905a303d0f46aad08ba17dfb042c3ea767) refactor(model): share secret text input
- [`889a504`](https://github.com/yelog/lazydb/commit/889a504ed83215406a94b80e560f06fdf2dfc3f8) refactor(mysql): centralize authoritative mutation definitions
- [`29ce3d2`](https://github.com/yelog/lazydb/commit/29ce3d27b98be034dda8539d449898b470648a6b) test(mariadb): add relation mutation round-trip fixture
- [`e4efade`](https://github.com/yelog/lazydb/commit/e4efade0b6a03628303b13c8b57e4c5b48a22d26) fix(mariadb): synchronize transaction and cancellation outcomes
- [`afc858f`](https://github.com/yelog/lazydb/commit/afc858fac5b5c5db49b807abf66c6bc74c31e80a) test(mariadb): certify value and result-set fidelity
- [`2763477`](https://github.com/yelog/lazydb/commit/276347735ee1dd973f1566f9212a0e6ff24b8fee) fix(mysql): preserve signed and extended TIME values
- [`0aa7e85`](https://github.com/yelog/lazydb/commit/0aa7e851f7f7b8b059052a5c3b6560627a7dbe7d) feat(mariadb): derive capabilities from server identity
- [`77ccfc7`](https://github.com/yelog/lazydb/commit/77ccfc7002ae654d081b0bee6aee25ade423cbfd) chore: save remaining workspace changes
- [`bcb9789`](https://github.com/yelog/lazydb/commit/bcb97891a23c72409a3d503f98b8c8463513e1b6) fix(mariadb): normalize product-specific server versions
- [`1fe7b48`](https://github.com/yelog/lazydb/commit/1fe7b482c6a7528c3f14828cb38dc21d4b4377c5) refactor(catalog): type database and role form focus
- [`4928d59`](https://github.com/yelog/lazydb/commit/4928d5945a888d9cfc37ed3fd3f8de78124c868a) test(mariadb): add explicit strict integration entrypoints
- [`c0ed2a0`](https://github.com/yelog/lazydb/commit/c0ed2a0b5a9be8b5a417420321d94fb25ea853cb) feat(redis): add focused value preview panel
- [`e0152b4`](https://github.com/yelog/lazydb/commit/e0152b4aa8951cf39904b9a3873910e71b174a20) fix(ui): preserve grid cell colors when selected
- [`bc88b7f`](https://github.com/yelog/lazydb/commit/bc88b7f280ba9eaf9922cc81e92d03948970de97) docs(redis): add key search implementation plan
- [`ecc0472`](https://github.com/yelog/lazydb/commit/ecc0472823477bf7b1d0cec0d93e40d1cf9954ae) merge: improve Redis key search
- [`a24c532`](https://github.com/yelog/lazydb/commit/a24c532eb30a8b98e4e8f8c9ee6938ae69b0331f) fix(redis): defer key search until enter
- [`c341618`](https://github.com/yelog/lazydb/commit/c34161813d4e0fa69dae96e87624248d44f46344) merge: add multi-database object mutation
- [`dee44c4`](https://github.com/yelog/lazydb/commit/dee44c43ff09554b83641b0aa35d5efe40931809) merge: optimize Redis preview performance
- [`0ee1d4b`](https://github.com/yelog/lazydb/commit/0ee1d4b45e32cbc74367bd8ea75bf89b717420c9) docs(redis): record preview optimization verification
- [`2dd0fca`](https://github.com/yelog/lazydb/commit/2dd0fca46ac476b666c0d5ce79d03d98247b6f34) perf(redis): prepare large value previews off the UI path
- [`5a77641`](https://github.com/yelog/lazydb/commit/5a77641d84a37ba5b462592f26b76d20aca5bd5f) perf(redis): debounce preview selection requests
- [`f58b7bf`](https://github.com/yelog/lazydb/commit/f58b7bf1eef97cc16c2ba91b6a54ce483d814841) perf(redis): reuse metadata during value preview reads
- [`9068116`](https://github.com/yelog/lazydb/commit/9068116a7ceff2c8fd9848c5f218dec6062983e1) perf(preview): isolate and share preview cache values
- [`73f09d1`](https://github.com/yelog/lazydb/commit/73f09d1eecf9e27f3007a20f009b0631b97d3f8b) feat(redis): add native mutation editor
- [`451c3df`](https://github.com/yelog/lazydb/commit/451c3df5f58e0b32d49dd7bc5cb4a689642166f6) perf(redis): avoid redundant preview rendering work
- [`17594c0`](https://github.com/yelog/lazydb/commit/17594c08bdd18738df7d5b12e1fa1258acec9707) perf(editor): bypass SQL analysis for value previews
- [`3941959`](https://github.com/yelog/lazydb/commit/39419598d9bf8d7474d5b14d58cac5f07c12994e) fix(redis): detect previews from original bytes
- [`56bff29`](https://github.com/yelog/lazydb/commit/56bff2992a427815517af09c36cfd1fd54788543) test(redis): reproduce unicode JSON preview regression
- [`07dc3e9`](https://github.com/yelog/lazydb/commit/07dc3e9c5d9f6d0cb9c57c01609ab87dd89adfed) merge: confirm Redis prefix key deletion
- [`55819a9`](https://github.com/yelog/lazydb/commit/55819a94d3747abd02906ca1e34106e443485f06) feat(redis): confirm prefix key deletion
- [`18e6d0b`](https://github.com/yelog/lazydb/commit/18e6d0b95c8f6a33e92f45af1db9fbe924d9d023) Merge branch 'task/sql-history-display-search'
- [`d1a7374`](https://github.com/yelog/lazydb/commit/d1a73743e68626cc9b54678d3fccd2849aeaadc7) feat(redis): add structured value preview
- [`e14061d`](https://github.com/yelog/lazydb/commit/e14061d11aa868700999c89c15cc78bd8419c40a) feat(sqlite): rebuild tables for structural edits
- [`a481982`](https://github.com/yelog/lazydb/commit/a481982a504e92975824ef64e72bb53306f13be3) feat(sqlite): support native table and view editing
- [`26e1c8a`](https://github.com/yelog/lazydb/commit/26e1c8a00c859bf138f642b8064d82b379f0dc57) feat(sqlserver): support table and view editing
- [`ef5e09c`](https://github.com/yelog/lazydb/commit/ef5e09c357cdbeae65ff6da3106afe0cfbdfef92) feat(mysql): support view definition editing
- [`5827474`](https://github.com/yelog/lazydb/commit/5827474f5080912857aa8aa89faccdc64fa08175) feat(mysql): support table definition loading and rename
- [`9bdfc32`](https://github.com/yelog/lazydb/commit/9bdfc3229bb768eff160e99eb7680fc0e8021648) fix(sql-history): improve search and SQL detail rendering
- [`085ed6d`](https://github.com/yelog/lazydb/commit/085ed6d375cc01416ce8a7d57430a3cc8db875a7) docs: record oracle catalog editing support
- [`02fe0d3`](https://github.com/yelog/lazydb/commit/02fe0d3c38b39cd605d18b233b2135b53983387a) feat(oracle): support table view and sequence editing
- [`7c5a37e`](https://github.com/yelog/lazydb/commit/7c5a37eca277614a25b503df19d0d4ad1bbe9ba7) feat(oracle): load table view and sequence definitions
- [`04d1203`](https://github.com/yelog/lazydb/commit/04d120337513b272d4d3a332ec1808a125bee09f) feat(oracle): load authoritative table definitions
- [`50cd882`](https://github.com/yelog/lazydb/commit/50cd882d771062b1d76811daf7ead681e4307004) docs: record initial multi-database mutation support
- [`51b3fb3`](https://github.com/yelog/lazydb/commit/51b3fb370712170e7012d0a7bb41e7b2436361c4) test(sqlite): verify table creation plan execution
- [`6d4d877`](https://github.com/yelog/lazydb/commit/6d4d87721250bf422ff6dc35b2a7dbd0b8ce95b2) test(help): provide active catalog capabilities in fixture
- [`5eaa63e`](https://github.com/yelog/lazydb/commit/5eaa63eae8730e5a166031322a50662248e567e6) feat(sqlite): define lossless table rebuild plans
- [`551a136`](https://github.com/yelog/lazydb/commit/551a136381426fbdbedc4b8b6d00def3683c2165) feat(sqlserver): support initial table and view creation
- [`c9d4d73`](https://github.com/yelog/lazydb/commit/c9d4d732b8035d5e4563f6cd21b6ac7a0d8cf734) feat(sqlite): support initial table and view creation
- [`998dbdb`](https://github.com/yelog/lazydb/commit/998dbdb65735362bf59c5e38ceefc54feb8a1026) test(redis): cover stream preview reply parsing
- [`e7cbc31`](https://github.com/yelog/lazydb/commit/e7cbc3147bc0d24c6d652230ef1503851ea05dd9) feat(preview): decode table cell details consistently
- [`7e71064`](https://github.com/yelog/lazydb/commit/7e71064e5e4668b2377369e9fe3fc832d6e270e3) fix(explorer): apply namespace model to create options
- [`ca2db71`](https://github.com/yelog/lazydb/commit/ca2db71df31646184c1f8a1aac449ade361d105a) test(mariadb): preserve engine-specific catalog version gates
- [`6eca762`](https://github.com/yelog/lazydb/commit/6eca762f7d4ad634401d27b5a66def25b0b74d6b) merge: optimize Omni relation results
- [`c93e2a3`](https://github.com/yelog/lazydb/commit/c93e2a39ecdd50f2ff85b3ec8e7bb7ec97193dbe) feat(mysql): support initial table and view creation
- [`bc69343`](https://github.com/yelog/lazydb/commit/bc69343e57139e8a303dcd4048f20abd39f686ff) fix(omni): optimize relation results and metadata layout
- [`24b7569`](https://github.com/yelog/lazydb/commit/24b75695f6a9f96541768c31665f004542b741b2) test(oracle): keep editing gated on loaded definitions
- [`0d8fcf8`](https://github.com/yelog/lazydb/commit/0d8fcf8485f4a336fa3eed0eb5a1725d828065ac) test(preview): cover automatic format selection
- [`7bf7558`](https://github.com/yelog/lazydb/commit/7bf7558699ac5adc9745a960e8f089a45fa102df) test(oracle): verify quoted catalog creation plans
- [`2e30ee6`](https://github.com/yelog/lazydb/commit/2e30ee610963ff039d212c0e7c75f2d8482dda56) feat(oracle): support initial catalog create plans
- [`522944b`](https://github.com/yelog/lazydb/commit/522944b3cb0840467ab01b288bb13fe733efcfa3) fix(help): dispatch SQL history shortcut correctly
- [`29c59d5`](https://github.com/yelog/lazydb/commit/29c59d5340d238379fef4418c7778864271249a1) feat(redis): add stream paging and cell details
- [`40bdf0e`](https://github.com/yelog/lazydb/commit/40bdf0eac0a5b571b494d37cd0a232a14b04fd20) feat(redis): support paged stream previews
- [`ae93b8a`](https://github.com/yelog/lazydb/commit/ae93b8a4d73479ed76535ee774c4c887aab89705) feat(runtime): model multi-step mutation outcomes
- [`7d1b953`](https://github.com/yelog/lazydb/commit/7d1b953bc4a6c4467596cde6a0881060cdac6f5a) refactor(editor): derive editable object types from adapter capability
- [`940c950`](https://github.com/yelog/lazydb/commit/940c950075ccd368a24be41a3745457778517c55) refactor(db): add lossless catalog change inputs
- [`c76e79b`](https://github.com/yelog/lazydb/commit/c76e79be6dbd31b43355a0c9a0f6a73c0172dc19) fix(explorer): align mutation entry points with adapter capabilities
- [`c1b4111`](https://github.com/yelog/lazydb/commit/c1b4111c398083034fed8361b05cc8687f69b7ad) chore(preview): keep incremental ui changes lint clean
- [`b71d9c2`](https://github.com/yelog/lazydb/commit/b71d9c2d52745884b067a075bbb5506cf50b78ff) refactor(explorer): centralize object action resolution
- [`f9ec63f`](https://github.com/yelog/lazydb/commit/f9ec63f02e29d3541e625af0ad183c272fde65f7) feat(redis): add paged table and preview loading
- [`cc3dab5`](https://github.com/yelog/lazydb/commit/cc3dab53074077ba8d5f525651eaefefe10526d6) refactor(db): model namespace-aware create options
- [`df1e7fd`](https://github.com/yelog/lazydb/commit/df1e7fdacc1035f772e742cc9979d60d3c15ccdc) docs: define multi-database object mutation scope
- [`4fc0448`](https://github.com/yelog/lazydb/commit/4fc044825a0af4ccfa928fa5ac4d32df78b46987) feat(redis): render table previews with isolated hits
- [`8f5c2ee`](https://github.com/yelog/lazydb/commit/8f5c2eef7d4bd1e48f404451adf579126bedecc7) merge: improve Redis keys navigation and mutations
- [`99d1a32`](https://github.com/yelog/lazydb/commit/99d1a32d35d687a3f571b6ce908fb8983b6490c3) feat(redis): improve keys navigation and mutations
- [`a937f1a`](https://github.com/yelog/lazydb/commit/a937f1a988f607d1e3483014b82c46a5b91ed51f) feat(preview): highlight json and yaml editor views
- [`c327a59`](https://github.com/yelog/lazydb/commit/c327a59239a75a59b4e05740aebe800140525bc5) feat(history): add SQL history modal
- [`9ea13e8`](https://github.com/yelog/lazydb/commit/9ea13e8c5e7bb42fe3816988ab07a88341b58579) docs(redis): document preview capabilities and validation
- [`927a2bc`](https://github.com/yelog/lazydb/commit/927a2bc68d632401cbe1691cdf843c1badc5439e) perf(preview): bound derived value cache
- [`2dd2252`](https://github.com/yelog/lazydb/commit/2dd2252b9c7875c15eab2a0b045c6771d68e4b8d) refactor(history): remove legacy history tab
- [`b307e3d`](https://github.com/yelog/lazydb/commit/b307e3d59d958d43553e54a881a34d2c00c8eae7) merge: fix Omni panel readability
- [`d7a8bd2`](https://github.com/yelog/lazydb/commit/d7a8bd2a744112b12c86443c8955131afae7bf6c) fix(ui): keep Omni panel readable
- [`62fddf9`](https://github.com/yelog/lazydb/commit/62fddf96a5020e8ed0a865a341aa3bb66e4f84f5) feat(preview): validate automatic format detection
- [`cd04a9f`](https://github.com/yelog/lazydb/commit/cd04a9f4e3d5dd2209a78767af0368101abd0291) feat(preview): add strict protobuf wire decoding
- [`79757d0`](https://github.com/yelog/lazydb/commit/79757d0f36b66985b3d014415e3c3af05bf0e09c) feat(preview): decode php and pickle values
- [`e6bd9c0`](https://github.com/yelog/lazydb/commit/e6bd9c0cca07508f6cb4e2f538d31df0807f52b4) feat(preview): decode java serialized values
- [`6a3d53e`](https://github.com/yelog/lazydb/commit/6a3d53e48d0ea3e6d36cef8fd854e2f66a19657f) feat(preview): preserve selected view across value pages
- [`deb5df0`](https://github.com/yelog/lazydb/commit/deb5df03a462e3fdef0ed4b2ad9210d3f2b0d0a6) feat(redis): add lossless table projection
- [`39de910`](https://github.com/yelog/lazydb/commit/39de91068911ef939ce255731b831cdb5f68d12e) fix(history): refine modal input modes
- [`76704a3`](https://github.com/yelog/lazydb/commit/76704a3ed4ab86b9439daed91a5c648d5602e8ce) feat(preview): format json and yaml views
- [`bf13fcc`](https://github.com/yelog/lazydb/commit/bf13fcc8d7d633de378442b1e7161462b7af2252) feat(preview): add raw format selector
- [`15187db`](https://github.com/yelog/lazydb/commit/15187db46a0860da57b7b8e906e92e338d743c02) test(history): cover modal lifecycle
- [`4889e14`](https://github.com/yelog/lazydb/commit/4889e14411582dd0b709272aa025cd0fc7d88920) feat(redis): add preview metadata header
- [`380cdf7`](https://github.com/yelog/lazydb/commit/380cdf70986fdfa984407aedbea4c51805273c94) fix(ui): preserve non-history overlays when dismissing
- [`6581abe`](https://github.com/yelog/lazydb/commit/6581abef41e7613d537527f668b0827d74ddb64f) feat(redis): reuse vim editor interactions in preview
- [`fc9c5c4`](https://github.com/yelog/lazydb/commit/fc9c5c4e3cfb9471255857e56e13f0f979f4709a) refactor(editor): add preview language contract
- [`e322d31`](https://github.com/yelog/lazydb/commit/e322d311eac504cfa4f51d9cfa1915e0716658ec) perf(history): use per-connection SQL preview dialect
- [`834b973`](https://github.com/yelog/lazydb/commit/834b973cde9ff85958d6d2a5f235f8a8f36a129f) feat(history): add modal search and pagination
- [`27945c0`](https://github.com/yelog/lazydb/commit/27945c0a08e6c7c048f6b01c104d403504cf489a) refactor(redis): introduce unified preview content state
- [`dc249c1`](https://github.com/yelog/lazydb/commit/dc249c137fbf91a01c18c237b602a8c2b62dd306) test(redis): verify metadata size semantics
- [`ad9e0cd`](https://github.com/yelog/lazydb/commit/ad9e0cd9b89ed6a112000876598bb373d679b20e) feat(redis): establish preview contracts and metadata
- [`f0197e6`](https://github.com/yelog/lazydb/commit/f0197e65869d538beb57bd06397079b33fa47364) feat(history): enter and select SQL in modal
- [`733d4a7`](https://github.com/yelog/lazydb/commit/733d4a7553033fdff8c492fde89505595ee0c4b9) feat(history): add highlighted master detail layout
- [`ef8507e`](https://github.com/yelog/lazydb/commit/ef8507eed2491f6730d58f2b680ccc91d90e469b) merge: add Redis dashboard monitoring
- [`b794e96`](https://github.com/yelog/lazydb/commit/b794e96341ccb900635040923475d078524a4606) refactor(ui): share read-only SQL rendering
- [`8140eca`](https://github.com/yelog/lazydb/commit/8140eca1cde0b5b67bf9fc82d992f5f395c4f59d) feat(redis): add Redis dashboard monitoring
- [`e3b122d`](https://github.com/yelog/lazydb/commit/e3b122d4cd723fe185939d6f74b5f8d6b6bb94d8) refactor(history): add SQL history overlay shell
- [`14318f8`](https://github.com/yelog/lazydb/commit/14318f88a8b5aa0217d27f02f3091c7a20b5c391) feat(history): add modal state model
- [`4ca179a`](https://github.com/yelog/lazydb/commit/4ca179a45301ccb8e6ef9b23a3cc8458a8040c57) style: format merged redis changes
- [`ceefae4`](https://github.com/yelog/lazydb/commit/ceefae4cae519014f45baea84dc748e18e554a0c) Merge branch 'task/redis-browser-ui-consistency'
- [`a84e740`](https://github.com/yelog/lazydb/commit/a84e7402cbd5313573bee3effafe915439cf2168) feat(redis): improve browser navigation and pane consistency
- [`68bc15a`](https://github.com/yelog/lazydb/commit/68bc15ae8236106785b46ce7459dcb3b79c75919) merge: resolve workspace save integration
- [`cfc9557`](https://github.com/yelog/lazydb/commit/cfc95572c360e0efb5e18cbae5f38097261332b9) fix(workspace): prevent duplicate tabs during save
- [`f2491ea`](https://github.com/yelog/lazydb/commit/f2491ea84a6edf1b2c609dae1a2a4ce78971af8f) docs(plans): add implementation plans
- [`3fc058d`](https://github.com/yelog/lazydb/commit/3fc058d7092b88d6081f073222efbe0c186a6cfd) feat(history): add F7 shortcut
- [`e0623a2`](https://github.com/yelog/lazydb/commit/e0623a22cbf48199e2bd50c7620d1017c7d1a105) merge: connection brand consistency
- [`bf53da0`](https://github.com/yelog/lazydb/commit/bf53da0f8a8afbb92facd0eeb21a566b707cc282) docs(plan): add connection brand consistency plan
- [`523005a`](https://github.com/yelog/lazydb/commit/523005a6e4eccfccc9cfd9fc4515eb8f4d6ae6fc) fix(ui): unify connection brand icons and tab colors
- [`d799bfc`](https://github.com/yelog/lazydb/commit/d799bfc1466dbc82a21ace187f4bc406eb396bf9) Merge branch 'task/multi-connection-workspace-consistency'
- [`22061b5`](https://github.com/yelog/lazydb/commit/22061b59675a28aaa2b4ebe85b291a03838e5940) fix(workspace): isolate multi-connection console state
- [`d35d76b`](https://github.com/yelog/lazydb/commit/d35d76b74a9424e3c763a28dd32faa4df91fd2e9) merge: integrate Omni usability improvements
- [`4a779bd`](https://github.com/yelog/lazydb/commit/4a779bdf58350f1b45f111adde1cef65c8bc4cf8) feat(omni): improve discovery and result presentation
- [`2dc464d`](https://github.com/yelog/lazydb/commit/2dc464d49ef3ce03f2519577ffa735253a018f8f) merge: optimize Redis browser implementation
- [`47b413e`](https://github.com/yelog/lazydb/commit/47b413e450bf8d93c774338b7735f4e0d642c3ea) fix(redis): keep staged key indexes consistent on refresh
- [`ce83cfb`](https://github.com/yelog/lazydb/commit/ce83cfb54b2092a520179f048db11b540b423cfb) feat(redis): dispatch typed value previews from key selection
- [`87462cf`](https://github.com/yelog/lazydb/commit/87462cf94a1731ca5e6e987d3c90b51f4d371672) test(redis): validate browser scale and document workflows
- [`a9c15af`](https://github.com/yelog/lazydb/commit/a9c15af02efbf97708d466317daaa62fde5a3ce0) fix(redis): recover target connections with explicit generations
- [`c746646`](https://github.com/yelog/lazydb/commit/c746646ffdebaeb6f15cb935ad289c73fc86b778) perf(redis): batch and cache visible key metadata
- [`163cedc`](https://github.com/yelog/lazydb/commit/163cedccdd82aef3a5a19be0d684fc48e511bc5d) perf(redis): debounce previews and bound request concurrency
- [`c350512`](https://github.com/yelog/lazydb/commit/c35051202050a819d602bd71715eca170b8217b5) feat(redis): add scan budget scheduler
- [`cf5591e`](https://github.com/yelog/lazydb/commit/cf5591e63ca0937596a21bc1b39eee1bb920f0d0) feat(redis): add temporary sqlite key index
- [`f88496c`](https://github.com/yelog/lazydb/commit/f88496cce35058b1517524a6b3cb4b58bd93a33c) refactor(redis): separate key storage from browser state
- [`7593f34`](https://github.com/yelog/lazydb/commit/7593f344884e9a09ce6523cf317a2b32fc1fcce5) feat(redis): render paginated typed value previews
- [`3f33844`](https://github.com/yelog/lazydb/commit/3f33844fe4e3cf424baa9a79b11d074dd11fa86d) merge: fix multi-connection workspace persistence
- [`bdfc5d7`](https://github.com/yelog/lazydb/commit/bdfc5d72ab7d23d13e377965b1928bdc63b1cc84) fix(workspace): prevent duplicate consoles across connections
- [`b191af3`](https://github.com/yelog/lazydb/commit/b191af319c9b04eb3dd37a1d0110884e7b6332b7) feat(redis): integrate identity-safe value page lifecycle
- [`88a43e1`](https://github.com/yelog/lazydb/commit/88a43e13a7fc0061482b5456fb37ecf3fdec7a3f) feat(redis): read typed value pages with partial metadata
- [`a62d19c`](https://github.com/yelog/lazydb/commit/a62d19c64e0b35e8c9a43688c2ed77776536f3c1) Merge remote-tracking branch 'origin/main'
- [`38ab8eb`](https://github.com/yelog/lazydb/commit/38ab8eb96f3ca4ed3599ccb993e16b4d447e37f8) Merge pull request #11 from yelog/dependabot/github_actions/Swatinem/rust-cache-6323deb102c322ba6fcbdcafc7e3dddab59af2b6
- [`b112f2b`](https://github.com/yelog/lazydb/commit/b112f2bf10645132afa72596f38bdb55333fc833) refactor(redis): define typed value pages and output budgets
- [`f7cc629`](https://github.com/yelog/lazydb/commit/f7cc6292b78632dfd0025f9a77a5cc2848c0b2cb) perf(redis): cache viewport row lookup during search
- [`a4a0a06`](https://github.com/yelog/lazydb/commit/a4a0a068bcacfc582349a42b04e06a0197da249c) perf(redis): update key trees incrementally
- [`a11866d`](https://github.com/yelog/lazydb/commit/a11866de0fb7c26f13f4ab4b3b1bf1868b34ad3c) fix(redis): preserve scan progress and pending batches
- [`e00230d`](https://github.com/yelog/lazydb/commit/e00230d64a75e9e75807737b624933f1efb5a97b) test(redis): establish browser budgets and protocol fixtures
- [`d935810`](https://github.com/yelog/lazydb/commit/d935810cdcfbbff0a148994f28773db9ab8f871c) merge: add Redis support and database browser
- [`8f9973c`](https://github.com/yelog/lazydb/commit/8f9973c997b172266b4e689f07ab8fa9f9ef8d8b) feat(redis): add Redis support and database browser
- [`a77bd27`](https://github.com/yelog/lazydb/commit/a77bd27a051147f96c2b578bb22d1a29b7bbd667) docs(redis): add implementation plans
- [`b1fdf45`](https://github.com/yelog/lazydb/commit/b1fdf4569b0ce1f157ad6f0aff0bc20ddf308df8) test: tolerate late disconnect events
- [`53dd472`](https://github.com/yelog/lazydb/commit/53dd4726a13cd4f7bfe6e4265b38f9cd726a60a9) fix(test): run mysql catalog search through connection
- [`268a1ce`](https://github.com/yelog/lazydb/commit/268a1ce44261e192bdc787fbbeb23320071cfea5) test(mysql): restore temporal metadata assertion
- [`837248d`](https://github.com/yelog/lazydb/commit/837248d31ad8a67d5c19b4cf224682155d127859) test: tolerate asynchronous CI events
- [`a3fcb42`](https://github.com/yelog/lazydb/commit/a3fcb424cc8f33bc473f111c03b1d302a5fc6a7e) test(mysql): accept MariaDB generated expression aliases
- [`0712dc3`](https://github.com/yelog/lazydb/commit/0712dc301e2486f7e834449dbc5acf331344a0ba) fix(mysql): preserve generated column expressions
- [`77c9e98`](https://github.com/yelog/lazydb/commit/77c9e986d231af07c13089e2a3dd54c3e5eaf80f) fix(mysql): normalize generated column metadata
- [`26aa16e`](https://github.com/yelog/lazydb/commit/26aa16e442b30053ecf1fdadf5c000e023c2743a) fix(mysql): distinguish generated defaults
- [`1880b2a`](https://github.com/yelog/lazydb/commit/1880b2ad42b9ee4859f155bfcaaa68cdbb85876c) fix(mysql): support MariaDB generated columns
- [`6163ceb`](https://github.com/yelog/lazydb/commit/6163ceb1c67e535724154a4c5fb6ed4cde4c523b) fix(mysql): preserve generated expression metadata
- [`ee41402`](https://github.com/yelog/lazydb/commit/ee414029d28d8a76dcd0c469b1bde8b0c9c6f058) fix(mysql): normalize default expressions
- [`e6bd3ca`](https://github.com/yelog/lazydb/commit/e6bd3caca044575b332bc28e7c26c39dae3fd7d5) fix(ci): stabilize concurrent MariaDB assertions
- [`94bbedf`](https://github.com/yelog/lazydb/commit/94bbedf59168182bc8b17a4f42b8d7b2dd354b18) fix(mysql): account for optional functional index
- [`951ae87`](https://github.com/yelog/lazydb/commit/951ae87194a3af9e717c01583da6ab0be04d0398) fix(mysql): normalize nullable column lengths
- [`991e4e6`](https://github.com/yelog/lazydb/commit/991e4e6a242637507bd4a8df8546336388e06437) fix(mysql): decode unsigned column lengths
- [`7b25a7a`](https://github.com/yelog/lazydb/commit/7b25a7a503d26e22382f365c4805f4cff4e14f13) fix(ci): handle concurrent session test events
- [`3904503`](https://github.com/yelog/lazydb/commit/3904503730596f2f068ff1dc0e1999f56c6514a4) Merge branch 'task/multi-connection-session-completion'
- [`f5a3c06`](https://github.com/yelog/lazydb/commit/f5a3c0612338c55f9aedac389e673ab551f4886c) feat(runtime): complete multi-connection session routing
- [`1adf40b`](https://github.com/yelog/lazydb/commit/1adf40b343154c51875a95c44a1eb948aa8e879b) fix(ci): stabilize MariaDB and offline console workflows
- [`bd111f5`](https://github.com/yelog/lazydb/commit/bd111f5b2a9f335aa40f6d834080af05dd76e5f0) merge: add SQL execution history
- [`9bea2d3`](https://github.com/yelog/lazydb/commit/9bea2d38deea86dee4ba405ee0f00e864a911cda) fix(ci): normalize MariaDB keys and render fixtures
- [`42477c4`](https://github.com/yelog/lazydb/commit/42477c4888f7c91e3d4443fe602bb48fe6917023) fix(ci): handle MariaDB metadata and empty workspace tests
- [`4c83fd2`](https://github.com/yelog/lazydb/commit/4c83fd2b4acb9bf608adbb6189f9c672c569546f) feat(history): cycle status and transaction filters
- [`9a92068`](https://github.com/yelog/lazydb/commit/9a92068927ab3cd0d8e525ce49a9897c4eb5fc4c) fix(ci): handle mysql constraint metadata and test setup
- [`40e749d`](https://github.com/yelog/lazydb/commit/40e749d62d1332fd4fc39e4ad4a6ccf40c740726) feat(history): navigate selected history executions
- [`e0be669`](https://github.com/yelog/lazydb/commit/e0be66963ed5742603063fbf8a5c0e9a0755042b) feat(history): add SQL text search to history panel
- [`13c2cd4`](https://github.com/yelog/lazydb/commit/13c2cd4532d16fb7f13aa899fbfa1d95e65efdb6) feat(history): carry filter state through history requests
- [`042700b`](https://github.com/yelog/lazydb/commit/042700bce0c8ab3b0b84315055abf2ec89fc495d) feat(history): add composable history filters
- [`851699a`](https://github.com/yelog/lazydb/commit/851699af7b82949ff0c36c240ffd1cf8df3d137f) test(mcp): stabilize profile startup failure assertion
- [`e41362c`](https://github.com/yelog/lazydb/commit/e41362c9b84044777fa82d7b7e379a24f4d0bc29) fix(history): migrate execution target snapshot columns
- [`52201d6`](https://github.com/yelog/lazydb/commit/52201d6c55047bab05f1169467bcae29c4c0a3e5) merge: add OpenCode MCP compatibility
- [`4faa8a5`](https://github.com/yelog/lazydb/commit/4faa8a57dcff448c453ab96f46cf995fa6ca1c89) feat(history): group automatic paginated executions
- [`0d5138c`](https://github.com/yelog/lazydb/commit/0d5138ce390496ebb039a9d5fe4287494a539cde) fix(mysql): coalesce optional MariaDB constraint ordinal
- [`b30c18b`](https://github.com/yelog/lazydb/commit/b30c18b4af9b708cd1f4565d55ddb29e89c23bef) feat(history): persist execution database snapshots
- [`e573c11`](https://github.com/yelog/lazydb/commit/e573c11ef1314164e0bb08df77f96897726cc6a2) fix: preserve editor sessions across merged workspaces
- [`48f320e`](https://github.com/yelog/lazydb/commit/48f320e5ff45a6305e2858aae40cb0d715f4e82a) fix(history): migrate duration column for existing stores
- [`ef10213`](https://github.com/yelog/lazydb/commit/ef10213b2e398495d08d570deb072f05f81e9500) feat(history): display execution timestamps and durations
- [`42ce18d`](https://github.com/yelog/lazydb/commit/42ce18d1c6041adee35117edaa6c037ac99e5485) merge: add global omni bar
- [`c73add1`](https://github.com/yelog/lazydb/commit/c73add119f500b13ca003c06353b0c03edd3ae1a) feat(history): select history rows with the mouse
- [`3aeb297`](https://github.com/yelog/lazydb/commit/3aeb2976764704d932a08fba4634b6ef86b0152b) feat: add global omni bar
- [`9b7205b`](https://github.com/yelog/lazydb/commit/9b7205bf4bb2bed2bd2b098097b860d7343713e2) docs(mcp): document OpenCode compatibility and diagnostics
- [`0d31cf3`](https://github.com/yelog/lazydb/commit/0d31cf3afa01bad8c810be26836143748fcc0ddf) feat(history): copy and inspect complete SQL
- [`8c482bc`](https://github.com/yelog/lazydb/commit/8c482bcf42e8f065d4b565fc8e3754ac1a21b8a2) test(mcp): add OpenCode compatibility fixtures
- [`921f85f`](https://github.com/yelog/lazydb/commit/921f85f87859fccda1ed0aab96d0635a1c1431dd) fix(mysql): normalize MariaDB constraint ordinals
- [`8c0283c`](https://github.com/yelog/lazydb/commit/8c0283c1cabf22d5205b7396219f9b480547769e) chore(deps): bump Swatinem/rust-cache
- [`dd63404`](https://github.com/yelog/lazydb/commit/dd634046ea84200c66ee0810ec9eefefdc7a8ac4) feat(history): load and render history list
- [`32cffe1`](https://github.com/yelog/lazydb/commit/32cffe14e3d1de1d2477a40da58b7825e96567d2) fix(mysql): normalize MariaDB index ordinal type
- [`03d50e7`](https://github.com/yelog/lazydb/commit/03d50e77b2be36c37bbf3924f166c13f5c3e72a7) fix(ci): support MariaDB metadata and profile saves
- [`c375cd9`](https://github.com/yelog/lazydb/commit/c375cd912fa3f421d88e4d86865814a946766201) feat(history): add singleton history workspace tab
- [`a97850e`](https://github.com/yelog/lazydb/commit/a97850ed496fc24dfdcc597bd2d99a297e29f986) feat(history): flush recorder during runtime shutdown
- [`7378e0a`](https://github.com/yelog/lazydb/commit/7378e0a85fa59b1fbb754c9e1d139df5698f98c4) fix(history): classify cancellation and interrupted queries
- [`fafdb09`](https://github.com/yelog/lazydb/commit/fafdb0957de66cb587dc586340181a6b330c8bdb) feat(history): track manual transaction outcomes
- [`1eec480`](https://github.com/yelog/lazydb/commit/1eec4808a31ec694582a2051ef7aff49258a3610) feat(mcp): expose LazyDB server and build identity
- [`f2f5cf6`](https://github.com/yelog/lazydb/commit/f2f5cf6b520b8ab8bc94e8d42aa8c958fea56e26) fix(mysql): use MariaDB-compatible index metadata query
- [`b40d075`](https://github.com/yelog/lazydb/commit/b40d0750849cae2afa8ccad4656a0584ba8299d9) feat(history): record automatic console queries
- [`ee4619d`](https://github.com/yelog/lazydb/commit/ee4619d6bfc9631cba0f8e9fc3ac5f4e9952b5d2) fix(ci): handle profile migration and MariaDB fixture
- [`2b3b669`](https://github.com/yelog/lazydb/commit/2b3b669109be4c2874975b051fe9a245b88bdcdb) refactor(db): expose structured history error snapshots
- [`5e0668f`](https://github.com/yelog/lazydb/commit/5e0668f8b3a6c1d7646f48d148028a0109416687) feat(mcp): probe stdio startup and tool discovery
- [`eace3b2`](https://github.com/yelog/lazydb/commit/eace3b2a33f2abb463062c5b8daf046d61191721) feat(history): add shared recording queue
- [`a2731e3`](https://github.com/yelog/lazydb/commit/a2731e30b57f05c5faf886ec6a440fb660356f7d) feat(history): persist execution history with sqlite
- [`94353eb`](https://github.com/yelog/lazydb/commit/94353eb9f23d4166b2f005c51fc309c89972c6aa) docs: add implementation plans
- [`2943a81`](https://github.com/yelog/lazydb/commit/2943a817b8532cafcd09b233ff39180535924d1a) feat(history): add execution history domain model
- [`15d9ef4`](https://github.com/yelog/lazydb/commit/15d9ef45aedf732bed9b2e63d96b5f9ca6429082) fix(mcp): report effective OpenCode configuration accurately
- [`93718c1`](https://github.com/yelog/lazydb/commit/93718c1067452c56334d906dc2574fbfd465dcc1) feat(mcp): add format-aware OpenCode setup
- [`cd7a2c7`](https://github.com/yelog/lazydb/commit/cd7a2c7bc26da3265e1b422b9fcaa7f10f3b3331) Merge branch 'task/multi-connection-consoles'
- [`de71087`](https://github.com/yelog/lazydb/commit/de71087d165cee69391285c78d9fd9fab59a93f0) feat(workspace): support concurrent database consoles
- [`d313718`](https://github.com/yelog/lazydb/commit/d3137185e2ffaaa41fe0c55c0fa0c7cce3e76332) feat(profiles): support unavailable connection profiles
- [`4dbd4b3`](https://github.com/yelog/lazydb/commit/4dbd4b3e8f953720033f6b53e1e3929205f5ac24) fix(mcp): model OpenCode runtime discovery and precedence
- [`9982691`](https://github.com/yelog/lazydb/commit/9982691666aba1e87779387f4668bee44a5cecde) feat(mcp): normalize OpenCode v1 and v2 server entries
- [`5129570`](https://github.com/yelog/lazydb/commit/51295700794c97390cfd9e155516a8762f4d88f7) test(mcp): establish stdio startup and catalog baseline
- [`77e03a2`](https://github.com/yelog/lazydb/commit/77e03a2a91c331f7d03be21c324d36d10d0b7970) merge: fix Oracle catalog page SQL
- [`2cbb48f`](https://github.com/yelog/lazydb/commit/2cbb48f1ec940b5f10e0c2a80f3df5c0735c5f6c) fix(oracle): repair catalog page SQL construction
- [`66b046d`](https://github.com/yelog/lazydb/commit/66b046db64cf05080bb27b4e588e082174259eb2) merge: show Oracle explorer object counts
- [`e7ffeca`](https://github.com/yelog/lazydb/commit/e7ffeca227f0f30fc0046f2f9d34c6fe4fb7f154) fix(oracle): show catalog object counts
- [`5bcb95d`](https://github.com/yelog/lazydb/commit/5bcb95d30fab3aaa44f4c61e20946807f67501e6) merge: fix Oracle query pagination
- [`8bb38a9`](https://github.com/yelog/lazydb/commit/8bb38a9d779317102c78d1459a7c8a2c87353e8f) fix(oracle): use Oracle-compatible result pagination
- [`d292b54`](https://github.com/yelog/lazydb/commit/d292b54d0f4407072430c4e6fd00f17a84a01005) merge: fix Oracle sequence catalog loading
- [`0f8a276`](https://github.com/yelog/lazydb/commit/0f8a276175e0e740f5aa19cc768e63a58309bb26) fix(oracle): load sequences with correct catalog columns
- [`e2968d5`](https://github.com/yelog/lazydb/commit/e2968d54be363e00c8ebeb493c9f9567d3e1f80b) feat(profile): preview incomplete Oracle connection URLs
- [`f3c8466`](https://github.com/yelog/lazydb/commit/f3c846669c648fd8524240cec33ddd6f07933cb2) merge: fix stale connection URLs
- [`6fedd89`](https://github.com/yelog/lazydb/commit/6fedd8903b5256472568b369d8bdda1682421233) fix(profile): invalidate stale connection URLs
- [`4613da7`](https://github.com/yelog/lazydb/commit/4613da7f68c7649038d4ea1522c6f32c2e2a64fe) Merge branch 'task/oracle-sql-execution'
- [`1b67431`](https://github.com/yelog/lazydb/commit/1b674314b5ecd5047bf9643cd04f3b6edcf1b2a6) fix(oracle): handle SQL terminators in native execution
- [`69d7664`](https://github.com/yelog/lazydb/commit/69d766423a62b975c79ecb5315e6aa5cd00cb300) merge: fix Oracle relation DDL
- [`91fa7d5`](https://github.com/yelog/lazydb/commit/91fa7d53a4ac98631037138f386106a83d931013) fix(oracle): load relation DDL through scoped adapter
- [`c737d9c`](https://github.com/yelog/lazydb/commit/c737d9cf2a751db0b1fe3e479db09ec875b9f6ff) fix(db): map MariaDB and Oracle targets
- [`0284d6c`](https://github.com/yelog/lazydb/commit/0284d6c3238a1203dc86e553629111c8759db156) merge: expand relational database drivers
- [`9c4c1b7`](https://github.com/yelog/lazydb/commit/9c4c1b7bd78bb4534a22dcf4b1248ccb2f9973d1) feat(db): expand relational database drivers
- [`26b089e`](https://github.com/yelog/lazydb/commit/26b089e5c5bb3b86fb04b9cf30cb09c1124fddd4) fix(help): fix relation data shortcuts
- [`ea1ed1a`](https://github.com/yelog/lazydb/commit/ea1ed1a7b4331210f898536cfd58206b1ccfc99b) fix(ci): satisfy newer clippy option zip

## [0.1.6] - 2026-09-20

### Added

- Added Redis value editing in browser tables, with filtering, copying, YAML-aware previews, edit modes, search, undo, and dedicated dialogs.
- Added relational Users & Roles browsing and management, including read-only DDL tabs and principal mutations.
- Expanded catalog and relation editing with column ordering, column creation, schema-owner selection, comment preservation, and MariaDB creation and insert support.
- Added transaction SQL review, focus-scoped tab reordering, consolidated Help and Omni panels, and clearer dialog actions.

### Changed

- Improved editor navigation, keyboard and mouse shortcuts, modified-cell highlighting, large-catalog explorer performance, and update progress feedback.
- Expanded MariaDB compatibility and JSON/geometry value previews.

### Fixed

- Fixed Redis edit-state and connection-binding issues, profile workspace activation, principal edit loading, cross-connection navigation, and database-specific catalog behavior.
- Prevented MySQL/MariaDB principal previews from exposing authentication hashes returned by `SHOW GRANTS`.

### Internal

- Expanded database fixtures, tests, documentation, CI coverage, and release-note tooling.

### Commits

- [`955501b`](https://github.com/yelog/lazydb/commit/955501b6fb195743540ba70320840dfa774526f0) merge: add table editor column row actions
- [`76162d6`](https://github.com/yelog/lazydb/commit/76162d6edc0b1e15ff1814f0ca2bf9ccc1b8eb2f) feat(ui): add table column row actions
- [`b467bd4`](https://github.com/yelog/lazydb/commit/b467bd409b963d6b24ca0d11c8afc21f29d9479a) merge: unify profile save shortcuts and scope navigation
- [`ee90adc`](https://github.com/yelog/lazydb/commit/ee90adc625ebb704df9e863cdfc21a75502ab6fc) fix(profiles): unify save action and form shortcuts
- [`f7d03d5`](https://github.com/yelog/lazydb/commit/f7d03d54ac463cbe1c322e738f9a6fe7ee2d773c) merge: unify dialog actions and shortcut hierarchy
- [`a209d8e`](https://github.com/yelog/lazydb/commit/a209d8ebf698cc6a3a8c2f7970e71cc4c5d2b1df) feat(ui): unify dialog actions and shortcut hierarchy
- [`bed1481`](https://github.com/yelog/lazydb/commit/bed148153685d02154d8c7890bf21e9a9f0671dc) Merge branch 'task/fix-principal-edit-loading-mariadb-access'
- [`c8da3e5`](https://github.com/yelog/lazydb/commit/c8da3e5fd01594fd1ea1266555d52f99a01c659c) docs: add dialog actions implementation plan
- [`590fbbc`](https://github.com/yelog/lazydb/commit/590fbbca8167c7f43458db3c872765f79eec9ca4) merge: fix principal edit loading and MariaDB access
- [`2f02f63`](https://github.com/yelog/lazydb/commit/2f02f63487530809591a0f2fc863d2eea7713df2) fix(principals): recover edit loading and limited MariaDB access
- [`9037713`](https://github.com/yelog/lazydb/commit/90377133aa529b6841d6a2712bde1e08ffdc0f99) merge: integrate users and roles management
- [`218ba4a`](https://github.com/yelog/lazydb/commit/218ba4a3bc17221eaf8d58b7f4d93cf7d0a4f4ad) feat: manage users and roles from explorer
- [`2a91556`](https://github.com/yelog/lazydb/commit/2a91556a642895a7b10d1231c71bf963971f53ec) fix(workspace): disambiguate principal tab kind
- [`5a41d82`](https://github.com/yelog/lazydb/commit/5a41d82a7d2ae8363cb197b78f3ecfddcf2fb274) merge: unify clickable popup shortcuts
- [`da41ca9`](https://github.com/yelog/lazydb/commit/da41ca9af606cd43753648864319cb50ed23807a) feat(ui): unify clickable popup shortcuts
- [`8f73bb7`](https://github.com/yelog/lazydb/commit/8f73bb7e68be166887f58ec74535df3059f0883b) merge: integrate catalog comment editing
- [`02e7c66`](https://github.com/yelog/lazydb/commit/02e7c66c17b133b96e0cc65911d77ff55ef1fb44) fix(catalog): preserve mysql and oracle comments
- [`c634324`](https://github.com/yelog/lazydb/commit/c634324b6cc3a94dbb4735349fc20c4516f23282) merge: hide users roles until connection opens
- [`e412716`](https://github.com/yelog/lazydb/commit/e4127169f68e95366b2bad71925896e08f060d1e) fix(explorer): hide principals until connection is open
- [`45e330c`](https://github.com/yelog/lazydb/commit/45e330cedd3b8da493d1c198329ec38d5e1f38b5) merge: integrate users-roles-ddl-browser
- [`3f04e6e`](https://github.com/yelog/lazydb/commit/3f04e6e1f5a6342ea6f90a28bba60ca9bc5c784d) feat(explorer): add Users & Roles browser with read-only DDL tabs
- [`d0f8693`](https://github.com/yelog/lazydb/commit/d0f8693766fb85e16525965a1bed448d57df3d0c) merge: integrate explorer many-nodes performance
- [`6ca7406`](https://github.com/yelog/lazydb/commit/6ca7406225ce325f650599f9f08d7e201250cc94) perf(explorer): cut redundant projection and formatting on large lists
- [`0f31ece`](https://github.com/yelog/lazydb/commit/0f31ecea273f4609b52ee575436f475fb63d7bc2) merge: integrate update center redesign
- [`ec0ddfe`](https://github.com/yelog/lazydb/commit/ec0ddfe19ac0d79e45d510b7f0daf167211d3ca2) feat(update): redesign update center status, actions, and progress
- [`1b45a1e`](https://github.com/yelog/lazydb/commit/1b45a1eb6261cd309edb0c859fadbcbda3a0982f) fix: stabilize profile workspace activation
- [`34b17c9`](https://github.com/yelog/lazydb/commit/34b17c9ad47f0827536eef86e64a97c1c6c8d6e6) Merge branch 'task/highlight-modified-column-cells'
- [`98f2f1b`](https://github.com/yelog/lazydb/commit/98f2f1b5e774f03871fb679ff33c68522da74894) docs: add modified column highlights plan
- [`a1a028a`](https://github.com/yelog/lazydb/commit/a1a028a8b6e1336f0a515c82645ac2c677fcaa21) merge: integrate modified column cell highlights
- [`7884b20`](https://github.com/yelog/lazydb/commit/7884b202c1b559aa4299cfada626907573b9d1bf) fix(catalog-editor): highlight modified column cells
- [`303058c`](https://github.com/yelog/lazydb/commit/303058ce01b77955379d7e2bf0bdebb3493388d1) docs: add implementation plans
- [`16b32bf`](https://github.com/yelog/lazydb/commit/16b32bfc29d11e5f503ec51feea4fa8d381e988a) merge: integrate postgres catalog editor fixes
- [`2e824a3`](https://github.com/yelog/lazydb/commit/2e824a3d226303f2ea638142f8b5d65ef19a503e) Merge branch 'main' into task/fix-postgres-add-column-and-explorer-edit-ownership
- [`a7d0e85`](https://github.com/yelog/lazydb/commit/a7d0e85502b10d7b1050c9c3d5f7a940c6a97934) fix(catalog): bind explorer edits and append postgres columns
- [`2ee009a`](https://github.com/yelog/lazydb/commit/2ee009a77545a2671ffac7c40a37478abbdf984b) Merge branch 'task/fix-redis-value-input-and-connection-binding'
- [`de69d5c`](https://github.com/yelog/lazydb/commit/de69d5c90b47b34205bd8d926d588b8db3633f82) Merge branch 'main' into task/fix-redis-value-input-and-connection-binding
- [`83e6913`](https://github.com/yelog/lazydb/commit/83e69136cbd82841f9bb6ed3495eb9ddb3ac8a05) docs(redis): add value input and connection implementation plan
- [`baf9cd6`](https://github.com/yelog/lazydb/commit/baf9cd6783b97222488c5f9e7361850bf98c6b8d) fix(redis): bind value saves to the tab connection
- [`3870144`](https://github.com/yelog/lazydb/commit/38701448c582cd05aba2c3db3c61fa9b1bd8e04c) fix(editor): handle vim character arguments before counts
- [`8ec2584`](https://github.com/yelog/lazydb/commit/8ec2584fc0e60f5240b33563c0e3531578128eeb) Merge branch 'task/catalog-editor-column-ordering'
- [`a2f69a3`](https://github.com/yelog/lazydb/commit/a2f69a31e42de96d219b7a140311c2df50c536d9) docs(catalog): add column ordering implementation plan
- [`6dbda68`](https://github.com/yelog/lazydb/commit/6dbda6868fda23b64c6b0ff6917305bb572c9339) merge: integrate catalog editor column ordering
- [`a2a32f9`](https://github.com/yelog/lazydb/commit/a2a32f9438f67c5be9fdb77b4c9f2108c6d2396d) feat(catalog): support column ordering edits
- [`80e5563`](https://github.com/yelog/lazydb/commit/80e556374376df56ee31ba52b292ac7c1d7e5864) merge: unify help and omni panels
- [`7f533c7`](https://github.com/yelog/lazydb/commit/7f533c7692616d6ecd8a91e459d6373fc31f9fce) feat(ui): unify help and omni panels
- [`84c5df4`](https://github.com/yelog/lazydb/commit/84c5df482680bdaad719afdf1e7719e3b02cb78f) Merge branch 'task/redis-value-dialog-overhaul'
- [`441eaa2`](https://github.com/yelog/lazydb/commit/441eaa2ecdecdaafda67a117e8c5d7a6d87da503) docs(redis): add value dialog implementation plan
- [`ac650d8`](https://github.com/yelog/lazydb/commit/ac650d8c47fc16f93de9ff897f1449dc22b15b15) Merge branch 'task/catalog-columns-navigation-shortcuts-layout'
- [`40582e5`](https://github.com/yelog/lazydb/commit/40582e5fe7dbfcab466b0ea497f7907cfea34b5d) docs(catalog): add columns navigation implementation plan
- [`a1bdb62`](https://github.com/yelog/lazydb/commit/a1bdb62cfa7d3207e95bbdd2b4a95cc8e351f362) feat(catalog): improve table column navigation and layout
- [`3aadb81`](https://github.com/yelog/lazydb/commit/3aadb810031741f4ec1e78b04f190ed7251ea7b0) Merge branch 'task/redis-value-dialog-overhaul'
- [`69e7204`](https://github.com/yelog/lazydb/commit/69e7204c5b46a0e6e244ca6be71dea5a00f4ca32) feat(redis): overhaul value dialogs
- [`7d92f52`](https://github.com/yelog/lazydb/commit/7d92f523d25a2cde39911e2ee7951d8fdc6645ee) Merge branch 'task/restore-empty-workspace-after-closing-all-tabs'
- [`7b3d680`](https://github.com/yelog/lazydb/commit/7b3d6807bcde46a25547d7019c30b74b6d1bbdeb) docs: add empty workspace implementation plan
- [`5a67033`](https://github.com/yelog/lazydb/commit/5a67033079cea19284676e8c7990f0313260b31c) Merge branch 'task/restore-empty-workspace-after-closing-all-tabs'
- [`71fb7e0`](https://github.com/yelog/lazydb/commit/71fb7e0a41f977d408666a91ca98b2ee1cb462cb) fix(ui): restore empty workspace after closing all tabs
- [`af23ff6`](https://github.com/yelog/lazydb/commit/af23ff65072f203b5266769cb9898203a09854b5) Merge branch 'task/tab-reorder-focus-scope'
- [`472c1b9`](https://github.com/yelog/lazydb/commit/472c1b983f71f43302763b5126499fadca84f1c5) docs(tabs): add tab reordering implementation plan
- [`7242e81`](https://github.com/yelog/lazydb/commit/7242e813f5b0c317084be0c66a6d41a63564223d) merge: add focus-scoped tab reordering
- [`f4cb02d`](https://github.com/yelog/lazydb/commit/f4cb02de2992a0b33079abe8d516e4b7a703e437) feat(tabs): add focus-scoped tab reordering
- [`fd294bd`](https://github.com/yelog/lazydb/commit/fd294bd7e8b56d06d0ad945358ae7f0c83ace8d7) Merge branch 'task/relation-cell-editor-column-metadata'
- [`6f17a01`](https://github.com/yelog/lazydb/commit/6f17a016a7aaa19a9d2fc250027d4c4ed62e015e) docs: add relation cell editor implementation plan
- [`f1b041e`](https://github.com/yelog/lazydb/commit/f1b041e95b599272f8376216952fc1dc69168166) feat(ui): show column metadata in cell editor
- [`2df3990`](https://github.com/yelog/lazydb/commit/2df399013fd12a65816715e38180b23f58f99100) docs: align product documentation with current features
- [`c6c12a5`](https://github.com/yelog/lazydb/commit/c6c12a5a632b75fb86eee0a9c0d157c036af678b) fix(ui): highlight inserted relation rows with green backgrounds
- [`16b05f4`](https://github.com/yelog/lazydb/commit/16b05f4e3d1d7b78d126891d2b751a0440fc7e07) docs: add implementation plans
- [`498b656`](https://github.com/yelog/lazydb/commit/498b6564cce4ffa89051eeac7a557e152adca7a0) merge: add transaction review SQL editor
- [`94d4f40`](https://github.com/yelog/lazydb/commit/94d4f4064f6b4b6b962133ae1820f6545fe6d528) feat(transaction): add vim SQL review preview
- [`42f2744`](https://github.com/yelog/lazydb/commit/42f274480ef295f78e4bb0b69faf9ef38f799459) merge: support MariaDB keyless grid inserts
- [`191f8a2`](https://github.com/yelog/lazydb/commit/191f8a2ad3b701fa407fcd5e9539da0aa13bbbda) fix(relation): support MariaDB keyless grid inserts
- [`527acb6`](https://github.com/yelog/lazydb/commit/527acb66edaf00779862e1831f4c1929c602c285) Merge branch 'task/mariadb-brand-icon'
- [`1978584`](https://github.com/yelog/lazydb/commit/19785849cf21a3b72e7ff01aded1cdfab5a73d91) docs: add MariaDB brand icon plan
- [`101bb25`](https://github.com/yelog/lazydb/commit/101bb254107aecfc07baa97aa7dbb15c85d89873) fix(ui): use dedicated MariaDB brand icon
- [`9e037fc`](https://github.com/yelog/lazydb/commit/9e037fcab270dc66664b6873641758550a055793) fix(ui): use dedicated MariaDB brand icon
- [`2b6321c`](https://github.com/yelog/lazydb/commit/2b6321cd319b0d5c362e9e2457f6ba437c3491f4) Merge branch 'task/fix-omni-cross-connection-navigation'
- [`96bb1d3`](https://github.com/yelog/lazydb/commit/96bb1d330c4a32ad9cc7610cc5c3217b1a90b872) docs(omni): add cross-connection navigation plan
- [`1761bd7`](https://github.com/yelog/lazydb/commit/1761bd71d4a00403c6948a3f147758a41822c2cf) merge: fix Omni cross-connection navigation
- [`8ffaec8`](https://github.com/yelog/lazydb/commit/8ffaec82b04fccb22805a77cb081d97c14d78bc3) fix(omni): open relations after connection reuse
- [`9cdd680`](https://github.com/yelog/lazydb/commit/9cdd680416c5f5d13e5b89da14fb9d6cac9534a1) docs: add MariaDB insert returning plan
- [`67a5642`](https://github.com/yelog/lazydb/commit/67a5642a47a54b44a068581da749c234a91d0b58) fix(mariadb): return inserted rows directly
- [`92c6bf7`](https://github.com/yelog/lazydb/commit/92c6bf758a3f3beaae74f3fb80a6f1985301e27a) Merge branch 'task/fix-mariadb-explorer-create-shortcut'
- [`30bcf6d`](https://github.com/yelog/lazydb/commit/30bcf6d8f28e5b3da20ac4e48625ddffd181bebb) docs: add MariaDB Explorer creation plan
- [`0df5d8a`](https://github.com/yelog/lazydb/commit/0df5d8a014fa387d516e1c80c3b781f5c9a997ed) merge: enable MariaDB Explorer catalog creation
- [`25968cb`](https://github.com/yelog/lazydb/commit/25968cbe6912b5d289ddeffc8062f80a4dc2ceb0) fix(mariadb): enable explorer catalog creation
- [`08c95d7`](https://github.com/yelog/lazydb/commit/08c95d767c8dd608b9e7f5f0712946d3bb9eccb0) Merge branch 'task/mariadb-json-geometry-preview'
- [`6108f95`](https://github.com/yelog/lazydb/commit/6108f95b689ff6f82fbdec05b38d9556af26f732) fix(mysql): preview MariaDB JSON and geometry values
- [`d7f85bc`](https://github.com/yelog/lazydb/commit/d7f85bc544ef2c8e4d5a72db93b820821640ea57) merge: synchronize Redis YAML preview format
- [`42bf9a4`](https://github.com/yelog/lazydb/commit/42bf9a46a3291c07c71647189d63cc3eae0dc8bf) fix(redis): synchronize YAML preview format changes
- [`df964c5`](https://github.com/yelog/lazydb/commit/df964c510a019cf1800c5c49034bf5ba493fb9ba) Merge branch 'task/redis-value-editor-modes-search-undo'
- [`1bd7503`](https://github.com/yelog/lazydb/commit/1bd750359dcdb7f1e7d3d0064cb6331d22221951) docs(redis): add value editor implementation plan
- [`a2b9c6d`](https://github.com/yelog/lazydb/commit/a2b9c6d39d0d2e3d15599fd0150fee7dafd21f78) fix(redis): improve value editor modes and undo
- [`050b141`](https://github.com/yelog/lazydb/commit/050b1412c052a44469ec9523cde86704985bf9f8) docs(redis): add preview clean-state plan
- [`0c58cbb`](https://github.com/yelog/lazydb/commit/0c58cbb49cae3512cfb10b1039cfd2808efedd34) fix(redis): clear stale preview edit state
- [`fe3134a`](https://github.com/yelog/lazydb/commit/fe3134adf47b8002cf88cc467f251a71585fb3bb) merge: mariadb catalog compatibility
- [`b74a748`](https://github.com/yelog/lazydb/commit/b74a74897c930218e65ea4f1c49c0a9175d01438) fix(mariadb): restore catalog discovery on 11.4
- [`6bc0c07`](https://github.com/yelog/lazydb/commit/6bc0c079c301d60eae2af86210c402cbc782d806) Merge branch 'task/redis-value-editing'
- [`9426a30`](https://github.com/yelog/lazydb/commit/9426a30a74943875b650f523a05b4e658512688b) feat(redis): edit values in browser tables
- [`a09dc6a`](https://github.com/yelog/lazydb/commit/a09dc6a1a37f957f6336b1516afd5a779dfb945d) fix(redis): avoid catalog discovery warning on connection test
- [`ba26e6d`](https://github.com/yelog/lazydb/commit/ba26e6d0f0085c21c90f8b827ae5721cf9cde81b) Merge branch 'task/decouple-explorer-connections-from-editor-targets'
- [`053c3df`](https://github.com/yelog/lazydb/commit/053c3dfb9268d4177e646d22fa749175c73b8020) docs: add editor target decoupling plan
- [`62364c9`](https://github.com/yelog/lazydb/commit/62364c9052553902c5a91e5098efd6929486802b) feat(mariadb): add Docker test database fixture
- [`4cac796`](https://github.com/yelog/lazydb/commit/4cac796f83b4add5beb111cd4e602e6ee17dbc39) merge: decouple explorer connections from editor targets
- [`c80c438`](https://github.com/yelog/lazydb/commit/c80c43828e3ceb76c1c1b1b036166a18388c7cfa) fix(editor): preserve targets across explorer connections
- [`cb5bd81`](https://github.com/yelog/lazydb/commit/cb5bd810515557461710b4fef6de8ac50c09aebb) fix(relation): remove row yank shortcut prompt
- [`b413286`](https://github.com/yelog/lazydb/commit/b413286c2fce57f185ef4275bec0e8a711797d1d) merge: copy relation data cells with y
- [`1bb73d3`](https://github.com/yelog/lazydb/commit/1bb73d35a74d0ce7df8fcaee556733a5675a82e6) fix(relation): copy cells with y in data view
- [`1e317fe`](https://github.com/yelog/lazydb/commit/1e317fe1d3569e4f440ee6fefa07b4010e6c08e2) Merge branch 'task/redis-browser-focus-auto-connect'
- [`0716a06`](https://github.com/yelog/lazydb/commit/0716a0698cef1988007f1899359dfce1e75e686b) fix(redis): connect browser tabs on focus
- [`2f856d3`](https://github.com/yelog/lazydb/commit/2f856d3090fc2bf89b6941759069312019079235) merge: redis value table filter and copy
- [`ca65efa`](https://github.com/yelog/lazydb/commit/ca65efac83a643df2dc95072b99a0cf407590215) feat(redis): filter and copy table values
- [`714eadc`](https://github.com/yelog/lazydb/commit/714eadc269b96b440950f5fe53d5942061665d5b) Merge branch 'task/improve-update-result-output'
- [`c0affc6`](https://github.com/yelog/lazydb/commit/c0affc6db5ece8c79978b22d55895cb6e71b8326) feat(update): improve result output
- [`4093b47`](https://github.com/yelog/lazydb/commit/4093b474b27bad145ff1313427e4171d647b62c4) ci(release): collapse commit lists in release notes
- [`1285bc0`](https://github.com/yelog/lazydb/commit/1285bc0bf315b0f83efa3489d8365effa41b07a7) fix(ci): satisfy release clippy checks

## [0.1.7] - 2026-09-23

### Added

- Added user and role access workflows for navigating principals, reviewing permissions, and managing grants and memberships.
- Added offline SQL console editing with target-aware execution, automatic connection URL parsing, and configurable key sequences.
- Added optional Kitty-aware pane navigation, resizing, and maximize synchronization, alongside Redis scan recovery.

### Changed

- Improved pane navigation and resizing, SQL completion and connection flows, principal overview interactions, help scrolling, and result-grid responsiveness.
- Refined workspace persistence, profile deletion behavior, PostgreSQL permission details, and status-bar presentation.

### Fixed

- Restored relation-row navigation and corrected workspace save, Redis reconnect, and principal access workflow behavior.

### Internal

- Expanded CI and release packaging checks, integration coverage, and user documentation.

### Commits

- [`da41f75`](https://github.com/yelog/lazydb/commit/da41f75) docs(plan): add access workflow implementation plan
- [`564bee3`](https://github.com/yelog/lazydb/commit/564bee3) feat(principal): improve access entry workflow
- [`6841cb5`](https://github.com/yelog/lazydb/commit/6841cb5) Merge branch 'main' into task/principal-overview-access-navigation
- [`4694462`](https://github.com/yelog/lazydb/commit/4694462) test(principal): cover access navigation interactions
- [`4b2ee87`](https://github.com/yelog/lazydb/commit/4b2ee87) feat(principal): complete overview access navigation
- [`cbc4128`](https://github.com/yelog/lazydb/commit/cbc4128) fix(tests): align render assertions with current layout
- [`fe55b79`](https://github.com/yelog/lazydb/commit/fe55b79) fix(tests): follow current workspace and pane behavior
- [`2a4afb1`](https://github.com/yelog/lazydb/commit/2a4afb1) fix(tests): align fixtures with workspace connection flow
- [`81a70ec`](https://github.com/yelog/lazydb/commit/81a70ec) fix(ci): repair failing integration test setup
- [`344d1f0`](https://github.com/yelog/lazydb/commit/344d1f0) Merge branch 'task/fix-bidirectional-smart-pane-resizing'
- [`5f74835`](https://github.com/yelog/lazydb/commit/5f74835) docs: add smart pane resizing plan
- [`0edaa99`](https://github.com/yelog/lazydb/commit/0edaa99) merge: fix bidirectional smart pane resizing
- [`7a962ff`](https://github.com/yelog/lazydb/commit/7a962ff) fix(panes): make smart resizing bidirectional
- [`51c77a8`](https://github.com/yelog/lazydb/commit/51c77a8) Merge branch 'task/principal-overview-access-navigation'
- [`d37353a`](https://github.com/yelog/lazydb/commit/d37353a) docs(principal): add access navigation plan
- [`c5b9206`](https://github.com/yelog/lazydb/commit/c5b9206) merge: improve principal overview access navigation
- [`ef7ba23`](https://github.com/yelog/lazydb/commit/ef7ba23) feat(principal): improve overview access navigation
- [`e2b0d34`](https://github.com/yelog/lazydb/commit/e2b0d34) merge: fix workspace save active tab dialog
- [`1f139d0`](https://github.com/yelog/lazydb/commit/1f139d0) fix(workspace): prevent invalid active tab save failures
- [`2c19622`](https://github.com/yelog/lazydb/commit/2c19622) feat(kitty): synchronize LazyDB pane maximize
- [`c6de978`](https://github.com/yelog/lazydb/commit/c6de978) feat(kitty): add smart maximize shortcut
- [`23c63a1`](https://github.com/yelog/lazydb/commit/23c63a1) Merge branch 'task/lazydb-kitty-smart-pane-resize'
- [`c6a77a0`](https://github.com/yelog/lazydb/commit/c6a77a0) feat: add smart pane resize across LazyDB and Kitty
- [`a2beff1`](https://github.com/yelog/lazydb/commit/a2beff1) merge: fix postgres principal permission details
- [`7621f9c`](https://github.com/yelog/lazydb/commit/7621f9c) fix(postgres): restore principal permission details
- [`b43731b`](https://github.com/yelog/lazydb/commit/b43731b) Merge branch 'task/unbound-console-completion-target-execution'
- [`3650c20`](https://github.com/yelog/lazydb/commit/3650c20) docs: add unbound console implementation plan
- [`7781fd8`](https://github.com/yelog/lazydb/commit/7781fd8) fix(completion): ignore single-character insert commands
- [`93fe215`](https://github.com/yelog/lazydb/commit/93fe215) merge: integrate unbound console execution
- [`a23f472`](https://github.com/yelog/lazydb/commit/a23f472) feat(console): run SQL after selecting unbound target
- [`5f8c149`](https://github.com/yelog/lazydb/commit/5f8c149) Merge branch 'task/unify-explorer-connection-creation-shortcuts'
- [`afc8e22`](https://github.com/yelog/lazydb/commit/afc8e22) docs: add Explorer connection shortcut plan
- [`32634c1`](https://github.com/yelog/lazydb/commit/32634c1) merge: unify Explorer connection creation shortcuts
- [`530b487`](https://github.com/yelog/lazydb/commit/530b487) fix(explorer): unify connection creation shortcuts
- [`a6b0097`](https://github.com/yelog/lazydb/commit/a6b0097) merge: add Kitty-aware smart pane navigation
- [`8fe3110`](https://github.com/yelog/lazydb/commit/8fe3110) feat(panes): add Kitty-aware smart navigation
- [`904f9d9`](https://github.com/yelog/lazydb/commit/904f9d9) merge: align consoles footer with omni
- [`3314a67`](https://github.com/yelog/lazydb/commit/3314a67) fix(ui): align console manager footer with omni
- [`7b76fe9`](https://github.com/yelog/lazydb/commit/7b76fe9) merge: configurable pane keybindings
- [`709ea46`](https://github.com/yelog/lazydb/commit/709ea46) feat(keybindings): support configurable pane shortcuts
- [`0edbec5`](https://github.com/yelog/lazydb/commit/0edbec5) merge: add connection URL auto-parse
- [`bc1c5a8`](https://github.com/yelog/lazydb/commit/bc1c5a8) feat(profiles): auto-parse connection URLs
- [`e93f366`](https://github.com/yelog/lazydb/commit/e93f366) Merge branch 'task/omni-help-mouse-scrollbar'
- [`c26a77f`](https://github.com/yelog/lazydb/commit/c26a77f) docs: add omni help mouse scrollbar plan
- [`fb011eb`](https://github.com/yelog/lazydb/commit/fb011eb) merge: add omni help mouse scrolling
- [`444c8d6`](https://github.com/yelog/lazydb/commit/444c8d6) feat(ui): add mouse scrolling to help panels
- [`6b2a656`](https://github.com/yelog/lazydb/commit/6b2a656) docs: add footer implementation plan
- [`fe31cde`](https://github.com/yelog/lazydb/commit/fe31cde) fix(ui): simplify footer and clarify update status
- [`aff7947`](https://github.com/yelog/lazydb/commit/aff7947) feat(ui): merge header into bottom status bar
- [`4526561`](https://github.com/yelog/lazydb/commit/4526561) fix(input): restore relation data row navigation
- [`fef2489`](https://github.com/yelog/lazydb/commit/fef2489) merge: optimize grid input responsiveness
- [`ac913fb`](https://github.com/yelog/lazydb/commit/ac913fb) perf(grid): improve large result input responsiveness
- [`4d51ce9`](https://github.com/yelog/lazydb/commit/4d51ce9) Merge branch 'task/redis-connection-manager-scan-recovery'
- [`2587c60`](https://github.com/yelog/lazydb/commit/2587c60) docs: add redis recovery implementation plan
- [`7c27994`](https://github.com/yelog/lazydb/commit/7c27994) merge: redis connection manager scan recovery
- [`90e1b3f`](https://github.com/yelog/lazydb/commit/90e1b3f) fix(redis): recover dropped scan connections
- [`f1c243c`](https://github.com/yelog/lazydb/commit/f1c243c) merge: users and roles permission workspace
- [`3e0f8a9`](https://github.com/yelog/lazydb/commit/3e0f8a9) feat(principals): add users and roles permission workspace
- [`23a638f`](https://github.com/yelog/lazydb/commit/23a638f) Merge branch 'main' into task/explicit-console-management
- [`61039c0`](https://github.com/yelog/lazydb/commit/61039c0) fix(console): prevent implicit consoles on connection
- [`a9c8f7f`](https://github.com/yelog/lazydb/commit/a9c8f7f) merge: console offline editing and lazy connections
- [`d3c747a`](https://github.com/yelog/lazydb/commit/d3c747a) feat(console): support offline editing and lazy connections
- [`dd9246d`](https://github.com/yelog/lazydb/commit/dd9246d) fix(profiles): preserve valid config when deleting connections
- [`b7c3c0d`](https://github.com/yelog/lazydb/commit/b7c3c0d) fix(release): install quality check components
- [`ef5b9ce`](https://github.com/yelog/lazydb/commit/ef5b9ce) fix(release): recover v0.1.6 packaging

## [0.1.8] - 2026-10-07

### Added

- Add non-interactive connection profile management with project/global scope, repeatable upserts, encrypted credentials, and JSON output. ([`2fba8bc`](https://github.com/yelog/lazydb/commit/2fba8bc96a6d16f8b7f38cebbeb4d369425d3a49))

### Changed

- Honor system proxy configuration for update requests and preserve actionable request errors. ([`3b57005`](https://github.com/yelog/lazydb/commit/3b57005558565b458d8c296e774a732691bc3490))

### Internal

- Reduce test-profile disk usage. ([`933b697`](https://github.com/yelog/lazydb/commit/933b697a10fd8a7afaba466c6e70c0eb345972f7))
- Stabilize runtime tests by draining pending app-flow events. ([`bc01be8`](https://github.com/yelog/lazydb/commit/bc01be88d9e0387adfe12bde63765a6284f045f8))

### Commits

- [`2fba8bc`](https://github.com/yelog/lazydb/commit/2fba8bc96a6d16f8b7f38cebbeb4d369425d3a49) feat(connections): add non-interactive profile management
- [`933b697`](https://github.com/yelog/lazydb/commit/933b697a10fd8a7afaba466c6e70c0eb345972f7) build: reduce test profile disk usage
- [`3b57005`](https://github.com/yelog/lazydb/commit/3b57005558565b458d8c296e774a732691bc3490) fix(update): honor system proxies and preserve request errors
- [`bc01be8`](https://github.com/yelog/lazydb/commit/bc01be88d9e0387adfe12bde63765a6284f045f8) test(runtime): drain pending app flow events

## Unreleased

- Add persistent Explorer connection groups, group membership, and profile ordering.

All notable changes to LazyDB are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project follows [Semantic Versioning](https://semver.org/).

## [0.1.0-beta.1] - 2026-08-30

### Added

- feat: implement LazyDB M0 foundation ([`074322d`](https://github.com/yelog/lazydb/commit/
074322d4bfe3b126a98471ee89347b7453dd622a))
- feat(security): add native secret store boundary ([`bb5e9e4`](https://github.com/yelog/lazydb/commit/
bb5e9e47811c30d10670fb90b1917f0909d7a94c))
- feat(profiles): add connection draft validation ([`cdce725`](https://github.com/yelog/lazydb/commit/
cdce725827873e63d9ce030f0385faa7884f8f2d))
- feat(profiles): add profile manager reducer ([`c37cf1f`](https://github.com/yelog/lazydb/commit/
c37cf1f1e1f312b8caca6154bd71c7db921241f3))
- feat(profiles): persist runtime profile changes ([`4fc1ab1`](https://github.com/yelog/lazydb/commit/
4fc1ab11079df71a7b381a7fa77fbcca630c893b))
- feat(profiles): add manager input controls ([`aaab930`](https://github.com/yelog/lazydb/commit/
aaab9300ab1723564d6e7c029f226341ff28e116))
- feat(ui): render connection profile manager ([`ccdc313`](https://github.com/yelog/lazydb/commit/
ccdc31301d50750e57462b7ca75eb49061d735a2))
- feat(profiles): open manager on first launch ([`b71bd7d`](https://github.com/yelog/lazydb/commit/
b71bd7d1fd636a7544e8adf078e1d2c5788f88ec))
- feat(sql): add editor transactions and completion ([`ee3270e`](https://github.com/yelog/lazydb/commit/
ee3270ee81fcb160ce771bbbbd8bb2c99eb29f74))
- feat(explorer): implement database explorer ([`044e201`](https://github.com/yelog/lazydb/commit/
044e2019f7ab13fd86b26719bcc8d008f4c6a8fd))
- feat(editor): add cursor styles and selection rendering ([`c208316`](https://github.com/yelog/lazydb/commit/
c2083166ec320bfcd7819864b3c0eef15e1dca9f))
- feat(transaction): expose editor controls ([`bb3bcd0`](https://github.com/yelog/lazydb/commit/
bb3bcd0c4f1a9a888559010fae5fe583cdc05564))
- feat(sql): add editor execution targets ([`fd5048f`](https://github.com/yelog/lazydb/commit/
fd5048f9baca409a0ce2ee558012f024a4cea023))
- feat(sql): add target selection commands ([`8cdf5ce`](https://github.com/yelog/lazydb/commit/
8cdf5ce0f7ccd35062f46e1f222d6867557253a1))
- feat(sql): switch connections with editor tabs ([`9f5e34b`](https://github.com/yelog/lazydb/commit/
9f5e34b969482f8ef124d71bb8a6ba3446970cd9))
- feat(workspace): persist sql editor snapshots ([`cab487f`](https://github.com/yelog/lazydb/commit/
cab487f2befa0efe5819a487c8086663463c49df))
- feat(workspace): load restored consoles at startup ([`6506aef`](https://github.com/yelog/lazydb/commit/
6506aef0c6cc45ba2aff93829f9d49ef2fe777b9))
- feat(sql): add connection target selector ([`e6c3040`](https://github.com/yelog/lazydb/commit/
e6c3040d309295840be97ed3f0fbb06d0fab8cf9))
- feat(workspace): debounce saves and configure schema ([`aef8bba`](https://github.com/yelog/lazydb/commit/
aef8bba7565327b22c7ed14ec20ae2e012e9ad40))
- feat(ui): move editor context into title ([`0c3513e`](https://github.com/yelog/lazydb/commit/
0c3513e6d98250ed4148238f0dd1d8b9178d8088))
- feat(explorer): complete database explorer and relation pages ([`942e729`](https://github.com/yelog/lazydb/commit/
942e7296d4dbe42abb8926ba94293ebe090916cc))
- feat(profiles): improve connection management ([`3adb069`](https://github.com/yelog/lazydb/commit/
3adb069a0a8ae72379358de5b5b789c8f1107c26))
- feat(editor): add execution target selector ([`d5207be`](https://github.com/yelog/lazydb/commit/
d5207befb98476448b957d3d1d6b5d5f3facb347))
- feat(relation): improve data preview controls ([`4c26bfa`](https://github.com/yelog/lazydb/commit/
4c26bfa89eadef04381230e5b001881fd89e82e5))
- feat(ui): add configurable terminal icons ([`afc366e`](https://github.com/yelog/lazydb/commit/
afc366e8cc0ed5fdf922551ceac3c8c1f170850c))
- feat(profiles): discover visible objects automatically ([`ffdcc1f`](https://github.com/yelog/lazydb/commit/
ffdcc1f564b3378f2819d218bc332332d5d5fcf2))
- feat(credentials): add local encrypted password storage ([`d7676cc`](https://github.com/yelog/lazydb/commit/
d7676cc36bd9561c6eb3e80fe35d08a00dfdb325))
- feat(sql): improve editor completion assistance ([`d172e04`](https://github.com/yelog/lazydb/commit/
d172e0456c0ec215b1e08161a29612fda1bc23f9))
- feat(ui): add icons to profile driver options ([`7b4c01e`](https://github.com/yelog/lazydb/commit/
7b4c01ef75b4fa2aaa9630b57d57f18475f5cd7d))
- feat(sql): unify result data grid and filtering ([`6560e64`](https://github.com/yelog/lazydb/commit/
6560e64cd1a669e64007b145cf426f4d5242a70c))
- feat(ui): move connection URL to fixed form section ([`d4fa4ac`](https://github.com/yelog/lazydb/commit/
d4fa4acd0a78c9acec6c6f1da2d67e715051c194))
- feat(help): add searchable shortcut palette ([`6d70c2a`](https://github.com/yelog/lazydb/commit/
6d70c2a5d47c99ee970edf6fd0195bf0e35eb15f))
- feat(sql): expose selected and current formatting ([`44cb1cd`](https://github.com/yelog/lazydb/commit/
44cb1cdb7e214909e5c120f58c18cccccd84da55))
- feat(sql): qualify relation completion candidates ([`7a86a5a`](https://github.com/yelog/lazydb/commit/
7a86a5ab5f794087f9b956d942bb637c4084cd8d))
- feat(sql): use editor target for completion ([`87af00f`](https://github.com/yelog/lazydb/commit/
87af00f9e71efce8d9999cc74a452b614fe5542a))
- feat(ui): add horizontal data grid scrolling ([`d8b2277`](https://github.com/yelog/lazydb/commit/
d8b22777035f8d12e046c5731adf3f7d5a66d8ed))
- feat: add transactional relation data editing ([`f2a17f2`](https://github.com/yelog/lazydb/commit/
f2a17f2ba8c205facc16d82b82cfd9c731692a2a))
- feat: add relation mutation types ([`5240404`](https://github.com/yelog/lazydb/commit/
52404049769d560bac4308b6a9bdb600f05b0649))
- feat(explorer): add catalog object search ([`34c0f43`](https://github.com/yelog/lazydb/commit/
34c0f439e2e4612e4444225815a6d7aa1dcc8dce))
- feat: manage SQL editor lifecycle ([`224f487`](https://github.com/yelog/lazydb/commit/
224f487b5d78084d952dec1df5975bc49c5b88d9))
- feat(ui): add Vim data grid navigation ([`c64f18e`](https://github.com/yelog/lazydb/commit/
c64f18e0feecac99c45d86090937633c14b8365c))
- feat(explorer): improve tree navigation and column ordering ([`a525c04`](https://github.com/yelog/lazydb/commit/
a525c04d1d3ed8ff6fffd918b3ebfa610b397932))
- feat: add relation DDL preview ([`1a22f00`](https://github.com/yelog/lazydb/commit/
1a22f00fed89db7e38ef4cdd6fbaa1f044accce9))
- feat(explorer): split visible and catalog search ([`493cf0d`](https://github.com/yelog/lazydb/commit/
493cf0d20712d81f3c6fd214f90f9aec073449df))
- feat(input): unify single-line editor controls ([`5c71155`](https://github.com/yelog/lazydb/commit/
5c7115579ee1800bd5c560c78ee04bf405bd153b))
- feat(explorer): move catalog search to frontend ([`02cdb61`](https://github.com/yelog/lazydb/commit/
02cdb61007c1da37714b0873b98a21d022234de1))
- feat(input): change quit shortcut to Ctrl+C ([`c91993f`](https://github.com/yelog/lazydb/commit/
c91993f47aa21a103693212f33eac8b42ea2cee0))
- feat(explorer): improve search start and metadata display ([`b8e52e7`](https://github.com/yelog/lazydb/commit/
b8e52e7314b2349781f01673e30ec521c680081a))
- feat(sql): cap ad-hoc select results at 500 rows ([`1295bf0`](https://github.com/yelog/lazydb/commit/
1295bf0f8ef12bf856f2ae834d418b700fb147e9))
- feat(explorer): pin selected ancestor rows ([`6219951`](https://github.com/yelog/lazydb/commit/
6219951d83f8cda37eab826d043df3cdb431229d))
- feat(results): add read-only record view ([`f3459b4`](https://github.com/yelog/lazydb/commit/
f3459b4cb1f080fc8296aa06825778fa44652ea9))
- feat(sql): add fuzzy identifier completion ([`b69385b`](https://github.com/yelog/lazydb/commit/
b69385b2f7187dc8b3e076f3627d0bfc5c3b48b8))
- feat(tabs): replace sequence numbers with context icons ([`2f24338`](https://github.com/yelog/lazydb/commit/
2f24338922d80d51098b8462d750c3d20e478dc2))

### Changed

- docs: design dynamic profile manager ([`7b54b9c`](https://github.com/yelog/lazydb/commit/
7b54b9cfdc608554e8e6023724c5888f69b5ecf9))
- docs: design SQL editor and transactions ([`b5de003`](https://github.com/yelog/lazydb/commit/
b5de0034e04af05e67b247fb9c1314da2e776a08))
- docs: plan dynamic profile manager implementation ([`20bd92c`](https://github.com/yelog/lazydb/commit/
20bd92ca86d306a216d11603d817d0e31639c4c2))
- docs: document dynamic connection profiles ([`713ed1e`](https://github.com/yelog/lazydb/commit/
713ed1ed0d98f92713c8124067cfcca107295547))
- docs(sql): design editor runtime context ([`a4cfd0d`](https://github.com/yelog/lazydb/commit/
a4cfd0d7439cacc579df8c94431677dd6ee77b0e))
- docs(sql): plan editor runtime context ([`a7d3ebb`](https://github.com/yelog/lazydb/commit/
a7d3ebbded4516be5232cbc9a1f7b628a133d0af))
- docs(editor): design keymap completion lifecycle ([`5253eeb`](https://github.com/yelog/lazydb/commit/
5253eeb1b2945e5ad82488f8442afbdb24483317))
- docs(editor): plan keymap completion lifecycle ([`a674ad7`](https://github.com/yelog/lazydb/commit/
a674ad7df18bd1739b0b3ae2162cee1126becfff))
- docs(completion): design accept cursor lifecycle ([`a2eece1`](https://github.com/yelog/lazydb/commit/
a2eece1affaa0ec483ef0de7d3bf7fda06eb7b3b))
- docs(completion): plan accept cursor lifecycle ([`3ad92cf`](https://github.com/yelog/lazydb/commit/
3ad92cfa714b6bcddea37565bcaaf3a0aa66179a))
- docs(ui): design editor context title ([`cc1270a`](https://github.com/yelog/lazydb/commit/
cc1270a2a286b71bae60d32f073d5c7c04749daf))
- docs(ui): plan editor context title ([`4205592`](https://github.com/yelog/lazydb/commit/
42055923a4cd67227681c7be95d7c1f3a5396813))
- docs(sql): design completion and formatting fixes ([`4a093ff`](https://github.com/yelog/lazydb/commit/
4a093ff4b69d97d11fb8a13c74b87b2a078d8c40))
- docs(sql): plan completion and formatting fixes ([`deb1fb1`](https://github.com/yelog/lazydb/commit/
deb1fb105af871855a337167ddde4e6390b66056))
- docs(sql): design execution output log ([`89c29b6`](https://github.com/yelog/lazydb/commit/
89c29b6c3f942dd8b5c118643f68b0b873b175ab))
- docs: add relation editing implementation plans ([`757f9cd`](https://github.com/yelog/lazydb/commit/
757f9cd70faf8a14358d244a0383d34426822f3f))
- docs: add identifier completion plans ([`50b05a9`](https://github.com/yelog/lazydb/commit/
50b05a9b272563a658b3b2be224b9f118238bc12))

### Fixed

- fix(runtime): bind commands to active connections ([`1c32c4d`](https://github.com/yelog/lazydb/commit/
1c32c4db633af15b2a1b1ede503a790656f5b39d))
- fix(editor): route vim input through console machines ([`7e086d2`](https://github.com/yelog/lazydb/commit/
7e086d2589b39fb5736ca496ec7f3272b831f1dc))
- fix(transaction): honor exit choices and scoped sql ([`36f925a`](https://github.com/yelog/lazydb/commit/
36f925adb42bd602c323c3b58a3c367c409b1562))
- fix(transaction): await begin and retire workers ([`2c6df8a`](https://github.com/yelog/lazydb/commit/
2c6df8ad0b511c32d180aeb9dbd6d4af60a4a72a))
- fix(transaction): reject stale worker commands ([`8b5b1ef`](https://github.com/yelog/lazydb/commit/
8b5b1ef20e42b1c7edc5c20eb3457604f9105645))
- fix(transaction): own cancellation and shutdown cleanup ([`6e3f47d`](https://github.com/yelog/lazydb/commit/
6e3f47d96ce9767e3ed34a1f5de1d6d037b6314f))
- fix(workspace): keep targets aligned after profile changes ([`64ede18`](https://github.com/yelog/lazydb/commit/
64ede183519c6d916a3121d965c587672440e2d4))
- fix(workspace): add single-writer lock and durable saves ([`4f598de`](https://github.com/yelog/lazydb/commit/
4f598de8fe2a9c1b009ca3d4d501e51eb3950561))
- fix(editor): preserve global keys and completion boundaries ([`a1105eb`](https://github.com/yelog/lazydb/commit/
a1105ebea17947b12ffec3dad932957303cc38e1))
- fix(completion): preserve accepted cursor lifecycle ([`4d8b1d4`](https://github.com/yelog/lazydb/commit/
4d8b1d4addcbc2251595c5b19eabbf9caef684e2))
- fix(profiles): persist new connection passwords by default ([`813a764`](https://github.com/yelog/lazydb/commit/
813a76419728a9872f13d8bfc9bd5c2312e7d1b9))
- fix(explorer): respect focus for relation shortcuts ([`9b7a32a`](https://github.com/yelog/lazydb/commit/
9b7a32a3f01b279a6e3ba91712cc47091bbf639d))
- fix(keymap): preserve leader shortcuts on relation tabs ([`92c24fb`](https://github.com/yelog/lazydb/commit/
92c24fb6ceb3a74427c04b1d4a0f2a2389347b76))
- fix(ui): align workspace tabs with main content ([`a192121`](https://github.com/yelog/lazydb/commit/
a192121a6e290412d9aef17cc7a6f04b8e933983))
- fix(profiles): stabilize visible object selection ([`8c6c266`](https://github.com/yelog/lazydb/commit/
8c6c266f75b07c427f2545d42c450d093dd90008))
- fix(sql): sync cursor style with editor mode ([`fdf04c0`](https://github.com/yelog/lazydb/commit/
fdf04c086ef7b6037c037cbd32ee4fa096c213a8))
- fix(ui): prevent pane focus flicker ([`4af5269`](https://github.com/yelog/lazydb/commit/
4af52693dad2582d99fb1aba66e4184e64a094ed))
- fix(sql): handle transaction toggle shortcut ([`1e33287`](https://github.com/yelog/lazydb/commit/
1e33287b72f41000cb7cfeaa17d2431001c9ae09))
- fix(sql): restrict completion to insert mode ([`b695fa4`](https://github.com/yelog/lazydb/commit/
b695fa48602fdf1410bd4df63793cc8e286cd0b8))
- fix(sql): recover exit after lost transaction connection ([`4b056f5`](https://github.com/yelog/lazydb/commit/
4b056f5f446445004df2d4325965e3c95c413fc7))
- fix(sql): resolve statements from internal whitespace ([`a3e6fcb`](https://github.com/yelog/lazydb/commit/
a3e6fcb1cd6367d1e41596acf4498c8f8cafd163))
- fix(ui): improve table preview formatting ([`09cd959`](https://github.com/yelog/lazydb/commit/
09cd95927aaf9b24764410454b7fee6cd83b104e))
- fix: expand profile after manual connection ([`cbdaf69`](https://github.com/yelog/lazydb/commit/
cbdaf697902a1e4103e845b37dca7a4a7c656641))
- fix(ui): restore relation pane focus navigation ([`6db1421`](https://github.com/yelog/lazydb/commit/
6db14219521c6c70f31bb6d34ae7cc441323460e))
- fix(ui): keep grid selection visible while scrolling ([`4c8aba4`](https://github.com/yelog/lazydb/commit/
4c8aba4c47ee8694fac1f5f28bd5b7cb7a8c231b))
- fix: preserve explorer search key routing ([`1230bdf`](https://github.com/yelog/lazydb/commit/
1230bdf4b0615b5b8b7e3e734827075393437c60))
- fix(ui): refresh filtered relation rows ([`d59f80b`](https://github.com/yelog/lazydb/commit/
d59f80b23500b9530c03bb7249a4c6647fc0c8de))
- fix(explorer): adapt find selection to viewport state ([`50c4acb`](https://github.com/yelog/lazydb/commit/
50c4acb29947dbd0731a5faa7f5ea0076a15e3e5))
- fix(ui): route DDL window focus commands ([`332baa3`](https://github.com/yelog/lazydb/commit/
332baa3e1a98cdd815e96f24a4e10dbce42ad3c8))
- fix(explorer): restore navigation after find confirmation ([`07e13b9`](https://github.com/yelog/lazydb/commit/
07e13b93bfdc1390eb2fc57516bc8716376c6862))
- fix(explorer): show column type in details ([`67e9a3c`](https://github.com/yelog/lazydb/commit/
67e9a3c40969fbbcf305e0439030dbaea10cea12))
- fix(editor): refine insert controls and completion lifecycle ([`479d226`](https://github.com/yelog/lazydb/commit/
479d2262a1ba1f2f65d8284959867640b4683415))
- fix(explorer): keep parent selection visible ([`e262147`](https://github.com/yelog/lazydb/commit/
e2621479daa723540fb567203e32aed450da9337))
- fix(connection): enforce ten-second connect timeout ([`2f96f9c`](https://github.com/yelog/lazydb/commit/
2f96f9c4c8cebd2dc75ae9377295ed479bf6ad51))
- fix(grid): clean up empty result rendering ([`047da11`](https://github.com/yelog/lazydb/commit/
047da11c52c200b204c5b7757bf6dbcd5412a7b4))
- fix(grid): remove header separator row ([`ece8f8b`](https://github.com/yelog/lazydb/commit/
ece8f8b50091584e4fabdfd144c50634d0620b55))
- fix(grid): keep row numbers muted ([`2c7ba09`](https://github.com/yelog/lazydb/commit/
2c7ba094965b8e3ca0665bf7a0d0cfd6fc144e9f))
- fix(results): accept data query completion with enter ([`f42ae8a`](https://github.com/yelog/lazydb/commit/
f42ae8aab9240f68df0a140e1f1f27c4445bdb8e))
- fix(ci): support current release runner dependencies ([`30cb1d2`](https://github.com/yelog/lazydb/commit/
30cb1d264eb3452151cccb5770e9093305cbf287))
- fix(ci): restore release quality gates ([`106a828`](https://github.com/yelog/lazydb/commit/
106a828a8a100cfb9c8cd2d7f31ceb80f80a79cd))
- fix(ci): handle current MySQL metadata and lints ([`eae3459`](https://github.com/yelog/lazydb/commit/
eae3459a8c2399d13afbeb6bee4dd8cbae9bad22))
- fix(ci): finish platform compatibility fixes ([`a0563b4`](https://github.com/yelog/lazydb/commit/
a0563b418eb953d2813dedb661b3a2f267e61cbb))
- fix(mysql): read catalog metadata by column position ([`9e64682`](https://github.com/yelog/lazydb/commit/
9e64682d8ee897858a882ec561725e02882524da))
- fix(mysql): avoid dynamic catalog column names ([`71caf64`](https://github.com/yelog/lazydb/commit/
71caf64cd27519b2bedbb1de1d04a5beaa62be2f))
- fix(ci): stabilize cross-platform release checks ([`0a5cd2e`](https://github.com/yelog/lazydb/commit/
0a5cd2e05689e4cfc1dc73f1b2cf061b9c993ed1))
- fix(ci): finish integration compatibility checks ([`44d70cf`](https://github.com/yelog/lazydb/commit/
44d70cfe91b3d29bf0b818b329f05d162a0e9faf))
- fix(mysql): normalize system variable decoding ([`9fb8aa9`](https://github.com/yelog/lazydb/commit/
9fb8aa950dcb42930bb34491032da8fdf4186ff3))
- fix(mysql): honor canonical name mode ([`5de3200`](https://github.com/yelog/lazydb/commit/
5de32004551a0709c7cc239fa8d616783582cd42))
- fix(mysql): canonicalize relation schema lookup ([`e65cd2f`](https://github.com/yelog/lazydb/commit/
e65cd2f8f94ee52fbb07b5db737f840f8955613f))
- fix(mysql): compare mirrored schemas canonically ([`3862160`](https://github.com/yelog/lazydb/commit/
3862160460555a0316221399a3330e52f3e532d8))
- fix(mysql): reuse verified relation DDL identity ([`6ce1d32`](https://github.com/yelog/lazydb/commit/
6ce1d32bfa6a0a90d5980781e3229c15620905db))
- fix(mysql): align DDL with requested catalog scope ([`eeb88cb`](https://github.com/yelog/lazydb/commit/
eeb88cbcd302f8cc8d7bba0d2a680a5f06d07854))
- fix(mysql): scope DDL children to verified database ([`17757a1`](https://github.com/yelog/lazydb/commit/
17757a1be9906fcc78acda4c59433ebea04aa054))
- fix(mysql): preserve relation scope identity ([`2aec7aa`](https://github.com/yelog/lazydb/commit/
2aec7aab2fc8f8fee9d9d0aad192d60fe1658b18))
- fix(mysql): preserve DDL child ordering ([`167c8e6`](https://github.com/yelog/lazydb/commit/167c8e6b0653a585d7089404468cba601530ff4e))

### Internal

- test(profiles): cover full lifecycle ([`d720c2e`](https://github.com/yelog/lazydb/commit/
d720c2e441e29e75eaa624211c5fe9acb1ab1694))
- test(editor): characterize modal input failures ([`cb155e9`](https://github.com/yelog/lazydb/commit/
cb155e9864439e783be778dc1be898e2b7e37f5d))
- merge: integrate sqleditor worktree into main ([`33c6928`](https://github.com/yelog/lazydb/commit/
33c6928dba64250713b6230acf3bcb4e36fb0664))
- merge: integrate relation preview controls ([`fb674d7`](https://github.com/yelog/lazydb/commit/
fb674d7d33088a4b3dde16e0b99c63264c854bb7))
- chore: ignore codegraph index ([`a03daeb`](https://github.com/yelog/lazydb/commit/
a03daeb7bfd9c89fb927ccd6ff16cfe41b3fb06d))
- style(ui): refine explorer hierarchy ([`3264f0b`](https://github.com/yelog/lazydb/commit/
3264f0b73c69fd44fbcbeb982bdbb6f01157c394))
- merge: integrate SQL editor completion assistance ([`a01734c`](https://github.com/yelog/lazydb/commit/
a01734c98a330fe9e995b59da6596c8bd8afd5f7))
- merge: integrate fixed connection URL section ([`8682ef4`](https://github.com/yelog/lazydb/commit/
8682ef4b46404492399b91cb150e99103f9026f1))
- merge: integrate searchable help palette ([`41345ed`](https://github.com/yelog/lazydb/commit/
41345ed7d54e40f71ccecf193984f7d18d1029e5))
- test(ui): cover qualified completion popup rendering ([`f07df25`](https://github.com/yelog/lazydb/commit/
f07df25727f843c07bf0632babc4990fce4f5a47))
- merge: integrate SQL editor completion and formatting fixes ([`4e47002`](https://github.com/yelog/lazydb/commit/
4e470028c0ae1fd0f852da5c81e7e9aeb5b87717))
- merge: integrate transactional relation data editing ([`d36630b`](https://github.com/yelog/lazydb/commit/
d36630bd15676c962c9bd66be614045a8ad76f51))
- merge: integrate relation pane focus navigation ([`ecdfc48`](https://github.com/yelog/lazydb/commit/
ecdfc48df43ad1c3fa9c82d4179a47102ba07965))
- Merge branch 'task/table-preview' ([`3d20edd`](https://github.com/yelog/lazydb/commit/
3d20eddec312e99a776f303974e97c9f0225c23b))
- merge: integrate SQL editor lifecycle ([`43c8bf6`](https://github.com/yelog/lazydb/commit/
43c8bf6f4e04e0779f8cc197de13d9b91e6f52c2))
- Merge branch 'task/relation-ddl' ([`ce77866`](https://github.com/yelog/lazydb/commit/
ce77866141ded383c9446c2c39d098bf88f7c345))
- merge: integrate Explorer dual search ([`99f5997`](https://github.com/yelog/lazydb/commit/
99f5997a6f47d965b82ad472b5fd349ce6d8bd85))
- Merge task/identifier-completion into main ([`9d7d47d`](https://github.com/yelog/lazydb/commit/
9d7d47dc4f1b4693c116fac77505ba3640ac7eb7))
- ci(release): add beta and stable distribution pipeline ([`d4d99d7`](https://github.com/yelog/lazydb/commit/
d4d99d754c8799ccc8ca5c59bebc7ba17d844d2d))

### Commits

- [`167c8e6`](https://github.com/yelog/lazydb/commit/167c8e6b0653a585d7089404468cba601530ff4e) fix(mysql): preserve DDL child ordering
- [`2aec7aa`](https://github.com/yelog/lazydb/commit/
2aec7aab2fc8f8fee9d9d0aad192d60fe1658b18) fix(mysql): preserve relation scope identity
- [`17757a1`](https://github.com/yelog/lazydb/commit/
17757a1be9906fcc78acda4c59433ebea04aa054) fix(mysql): scope DDL children to verified database
- [`eeb88cb`](https://github.com/yelog/lazydb/commit/
eeb88cbcd302f8cc8d7bba0d2a680a5f06d07854) fix(mysql): align DDL with requested catalog scope
- [`6ce1d32`](https://github.com/yelog/lazydb/commit/
6ce1d32bfa6a0a90d5980781e3229c15620905db) fix(mysql): reuse verified relation DDL identity
- [`3862160`](https://github.com/yelog/lazydb/commit/
3862160460555a0316221399a3330e52f3e532d8) fix(mysql): compare mirrored schemas canonically
- [`e65cd2f`](https://github.com/yelog/lazydb/commit/
e65cd2f8f94ee52fbb07b5db737f840f8955613f) fix(mysql): canonicalize relation schema lookup
- [`5de3200`](https://github.com/yelog/lazydb/commit/
5de32004551a0709c7cc239fa8d616783582cd42) fix(mysql): honor canonical name mode
- [`9fb8aa9`](https://github.com/yelog/lazydb/commit/
9fb8aa950dcb42930bb34491032da8fdf4186ff3) fix(mysql): normalize system variable decoding
- [`44d70cf`](https://github.com/yelog/lazydb/commit/
44d70cfe91b3d29bf0b818b329f05d162a0e9faf) fix(ci): finish integration compatibility checks
- [`0a5cd2e`](https://github.com/yelog/lazydb/commit/
0a5cd2e05689e4cfc1dc73f1b2cf061b9c993ed1) fix(ci): stabilize cross-platform release checks
- [`71caf64`](https://github.com/yelog/lazydb/commit/
71caf64cd27519b2bedbb1de1d04a5beaa62be2f) fix(mysql): avoid dynamic catalog column names
- [`9e64682`](https://github.com/yelog/lazydb/commit/
9e64682d8ee897858a882ec561725e02882524da) fix(mysql): read catalog metadata by column position
- [`a0563b4`](https://github.com/yelog/lazydb/commit/
a0563b418eb953d2813dedb661b3a2f267e61cbb) fix(ci): finish platform compatibility fixes
- [`eae3459`](https://github.com/yelog/lazydb/commit/
eae3459a8c2399d13afbeb6bee4dd8cbae9bad22) fix(ci): handle current MySQL metadata and lints
- [`106a828`](https://github.com/yelog/lazydb/commit/
106a828a8a100cfb9c8cd2d7f31ceb80f80a79cd) fix(ci): restore release quality gates
- [`2f24338`](https://github.com/yelog/lazydb/commit/
2f24338922d80d51098b8462d750c3d20e478dc2) feat(tabs): replace sequence numbers with context icons
- [`30cb1d2`](https://github.com/yelog/lazydb/commit/
30cb1d264eb3452151cccb5770e9093305cbf287) fix(ci): support current release runner dependencies
- [`d4d99d7`](https://github.com/yelog/lazydb/commit/
d4d99d754c8799ccc8ca5c59bebc7ba17d844d2d) ci(release): add beta and stable distribution pipeline
- [`f42ae8a`](https://github.com/yelog/lazydb/commit/
f42ae8aab9240f68df0a140e1f1f27c4445bdb8e) fix(results): accept data query completion with enter
- [`2c7ba09`](https://github.com/yelog/lazydb/commit/
2c7ba094965b8e3ca0665bf7a0d0cfd6fc144e9f) fix(grid): keep row numbers muted
- [`9d7d47d`](https://github.com/yelog/lazydb/commit/
9d7d47dc4f1b4693c116fac77505ba3640ac7eb7) Merge task/identifier-completion into main
- [`b69385b`](https://github.com/yelog/lazydb/commit/
b69385b2f7187dc8b3e076f3627d0bfc5c3b48b8) feat(sql): add fuzzy identifier completion
- [`50b05a9`](https://github.com/yelog/lazydb/commit/
50b05a9b272563a658b3b2be224b9f118238bc12) docs: add identifier completion plans
- [`f3459b4`](https://github.com/yelog/lazydb/commit/
f3459b4cb1f080fc8296aa06825778fa44652ea9) feat(results): add read-only record view
- [`ece8f8b`](https://github.com/yelog/lazydb/commit/
ece8f8b50091584e4fabdfd144c50634d0620b55) fix(grid): remove header separator row
- [`047da11`](https://github.com/yelog/lazydb/commit/
047da11c52c200b204c5b7757bf6dbcd5412a7b4) fix(grid): clean up empty result rendering
- [`2f96f9c`](https://github.com/yelog/lazydb/commit/
2f96f9c4c8cebd2dc75ae9377295ed479bf6ad51) fix(connection): enforce ten-second connect timeout
- [`e262147`](https://github.com/yelog/lazydb/commit/
e2621479daa723540fb567203e32aed450da9337) fix(explorer): keep parent selection visible
- [`6219951`](https://github.com/yelog/lazydb/commit/
6219951d83f8cda37eab826d043df3cdb431229d) feat(explorer): pin selected ancestor rows
- [`1295bf0`](https://github.com/yelog/lazydb/commit/
1295bf0f8ef12bf856f2ae834d418b700fb147e9) feat(sql): cap ad-hoc select results at 500 rows
- [`b8e52e7`](https://github.com/yelog/lazydb/commit/
b8e52e7314b2349781f01673e30ec521c680081a) feat(explorer): improve search start and metadata display
- [`c91993f`](https://github.com/yelog/lazydb/commit/
c91993f47aa21a103693212f33eac8b42ea2cee0) feat(input): change quit shortcut to Ctrl+C
- [`479d226`](https://github.com/yelog/lazydb/commit/
479d2262a1ba1f2f65d8284959867640b4683415) fix(editor): refine insert controls and completion lifecycle
- [`02cdb61`](https://github.com/yelog/lazydb/commit/
02cdb61007c1da37714b0873b98a21d022234de1) feat(explorer): move catalog search to frontend
- [`5c71155`](https://github.com/yelog/lazydb/commit/
5c7115579ee1800bd5c560c78ee04bf405bd153b) feat(input): unify single-line editor controls
- [`67e9a3c`](https://github.com/yelog/lazydb/commit/
67e9a3c40969fbbcf305e0439030dbaea10cea12) fix(explorer): show column type in details
- [`07e13b9`](https://github.com/yelog/lazydb/commit/
07e13b93bfdc1390eb2fc57516bc8716376c6862) fix(explorer): restore navigation after find confirmation
- [`332baa3`](https://github.com/yelog/lazydb/commit/
332baa3e1a98cdd815e96f24a4e10dbce42ad3c8) fix(ui): route DDL window focus commands
- [`50c4acb`](https://github.com/yelog/lazydb/commit/
50c4acb29947dbd0731a5faa7f5ea0076a15e3e5) fix(explorer): adapt find selection to viewport state
- [`99f5997`](https://github.com/yelog/lazydb/commit/
99f5997a6f47d965b82ad472b5fd349ce6d8bd85) merge: integrate Explorer dual search
- [`493cf0d`](https://github.com/yelog/lazydb/commit/
493cf0d20712d81f3c6fd214f90f9aec073449df) feat(explorer): split visible and catalog search
- [`ce77866`](https://github.com/yelog/lazydb/commit/
ce77866141ded383c9446c2c39d098bf88f7c345) Merge branch 'task/relation-ddl'
- [`1a22f00`](https://github.com/yelog/lazydb/commit/
1a22f00fed89db7e38ef4cdd6fbaa1f044accce9) feat: add relation DDL preview
- [`a525c04`](https://github.com/yelog/lazydb/commit/
a525c04d1d3ed8ff6fffd918b3ebfa610b397932) feat(explorer): improve tree navigation and column ordering
- [`d59f80b`](https://github.com/yelog/lazydb/commit/
d59f80b23500b9530c03bb7249a4c6647fc0c8de) fix(ui): refresh filtered relation rows
- [`c64f18e`](https://github.com/yelog/lazydb/commit/
c64f18e0feecac99c45d86090937633c14b8365c) feat(ui): add Vim data grid navigation
- [`1230bdf`](https://github.com/yelog/lazydb/commit/
1230bdf4b0615b5b8b7e3e734827075393437c60) fix: preserve explorer search key routing
- [`43c8bf6`](https://github.com/yelog/lazydb/commit/
43c8bf6f4e04e0779f8cc197de13d9b91e6f52c2) merge: integrate SQL editor lifecycle
- [`224f487`](https://github.com/yelog/lazydb/commit/
224f487b5d78084d952dec1df5975bc49c5b88d9) feat: manage SQL editor lifecycle
- [`3d20edd`](https://github.com/yelog/lazydb/commit/
3d20eddec312e99a776f303974e97c9f0225c23b) Merge branch 'task/table-preview'
- [`4c8aba4`](https://github.com/yelog/lazydb/commit/
4c8aba4c47ee8694fac1f5f28bd5b7cb7a8c231b) fix(ui): keep grid selection visible while scrolling
- [`ecdfc48`](https://github.com/yelog/lazydb/commit/
ecdfc48df43ad1c3fa9c82d4179a47102ba07965) merge: integrate relation pane focus navigation
- [`34c0f43`](https://github.com/yelog/lazydb/commit/
34c0f439e2e4612e4444225815a6d7aa1dcc8dce) feat(explorer): add catalog object search
- [`6db1421`](https://github.com/yelog/lazydb/commit/
6db14219521c6c70f31bb6d34ae7cc441323460e) fix(ui): restore relation pane focus navigation
- [`cbdaf69`](https://github.com/yelog/lazydb/commit/
cbdaf697902a1e4103e845b37dca7a4a7c656641) fix: expand profile after manual connection
- [`09cd959`](https://github.com/yelog/lazydb/commit/
09cd95927aaf9b24764410454b7fee6cd83b104e) fix(ui): improve table preview formatting
- [`d36630b`](https://github.com/yelog/lazydb/commit/
d36630bd15676c962c9bd66be614045a8ad76f51) merge: integrate transactional relation data editing
- [`5240404`](https://github.com/yelog/lazydb/commit/
52404049769d560bac4308b6a9bdb600f05b0649) feat: add relation mutation types
- [`f2a17f2`](https://github.com/yelog/lazydb/commit/
f2a17f2ba8c205facc16d82b82cfd9c731692a2a) feat: add transactional relation data editing
- [`757f9cd`](https://github.com/yelog/lazydb/commit/
757f9cd70faf8a14358d244a0383d34426822f3f) docs: add relation editing implementation plans
- [`a3e6fcb`](https://github.com/yelog/lazydb/commit/
a3e6fcb1cd6367d1e41596acf4498c8f8cafd163) fix(sql): resolve statements from internal whitespace
- [`d8b2277`](https://github.com/yelog/lazydb/commit/
d8b22777035f8d12e046c5731adf3f7d5a66d8ed) feat(ui): add horizontal data grid scrolling
- [`4e47002`](https://github.com/yelog/lazydb/commit/
4e470028c0ae1fd0f852da5c81e7e9aeb5b87717) merge: integrate SQL editor completion and formatting fixes
- [`4b056f5`](https://github.com/yelog/lazydb/commit/
4b056f5f446445004df2d4325965e3c95c413fc7) fix(sql): recover exit after lost transaction connection
- [`f07df25`](https://github.com/yelog/lazydb/commit/
f07df25727f843c07bf0632babc4990fce4f5a47) test(ui): cover qualified completion popup rendering
- [`87af00f`](https://github.com/yelog/lazydb/commit/
87af00f9e71efce8d9999cc74a452b614fe5542a) feat(sql): use editor target for completion
- [`7a86a5a`](https://github.com/yelog/lazydb/commit/
7a86a5ab5f794087f9b956d942bb637c4084cd8d) feat(sql): qualify relation completion candidates
- [`44cb1cd`](https://github.com/yelog/lazydb/commit/
44cb1cdb7e214909e5c120f58c18cccccd84da55) feat(sql): expose selected and current formatting
- [`b695fa4`](https://github.com/yelog/lazydb/commit/
b695fa48602fdf1410bd4df63793cc8e286cd0b8) fix(sql): restrict completion to insert mode
- [`89c29b6`](https://github.com/yelog/lazydb/commit/
89c29b6c3f942dd8b5c118643f68b0b873b175ab) docs(sql): design execution output log
- [`deb1fb1`](https://github.com/yelog/lazydb/commit/
deb1fb105af871855a337167ddde4e6390b66056) docs(sql): plan completion and formatting fixes
- [`1e33287`](https://github.com/yelog/lazydb/commit/
1e33287b72f41000cb7cfeaa17d2431001c9ae09) fix(sql): handle transaction toggle shortcut
- [`4a093ff`](https://github.com/yelog/lazydb/commit/
4a093ff4b69d97d11fb8a13c74b87b2a078d8c40) docs(sql): design completion and formatting fixes
- [`41345ed`](https://github.com/yelog/lazydb/commit/
41345ed7d54e40f71ccecf193984f7d18d1029e5) merge: integrate searchable help palette
- [`6d70c2a`](https://github.com/yelog/lazydb/commit/
6d70c2a5d47c99ee970edf6fd0195bf0e35eb15f) feat(help): add searchable shortcut palette
- [`8682ef4`](https://github.com/yelog/lazydb/commit/
8682ef4b46404492399b91cb150e99103f9026f1) merge: integrate fixed connection URL section
- [`d4fa4ac`](https://github.com/yelog/lazydb/commit/
d4fa4acd0a78c9acec6c6f1da2d67e715051c194) feat(ui): move connection URL to fixed form section
- [`6560e64`](https://github.com/yelog/lazydb/commit/
6560e64cd1a669e64007b145cf426f4d5242a70c) feat(sql): unify result data grid and filtering
- [`7b4c01e`](https://github.com/yelog/lazydb/commit/
7b4c01ef75b4fa2aaa9630b57d57f18475f5cd7d) feat(ui): add icons to profile driver options
- [`4af5269`](https://github.com/yelog/lazydb/commit/
4af52693dad2582d99fb1aba66e4184e64a094ed) fix(ui): prevent pane focus flicker
- [`fdf04c0`](https://github.com/yelog/lazydb/commit/
fdf04c086ef7b6037c037cbd32ee4fa096c213a8) fix(sql): sync cursor style with editor mode
- [`a01734c`](https://github.com/yelog/lazydb/commit/
a01734c98a330fe9e995b59da6596c8bd8afd5f7) merge: integrate SQL editor completion assistance
- [`d172e04`](https://github.com/yelog/lazydb/commit/
d172e0456c0ec215b1e08161a29612fda1bc23f9) feat(sql): improve editor completion assistance
- [`d7676cc`](https://github.com/yelog/lazydb/commit/
d7676cc36bd9561c6eb3e80fe35d08a00dfdb325) feat(credentials): add local encrypted password storage
- [`3264f0b`](https://github.com/yelog/lazydb/commit/
3264f0b73c69fd44fbcbeb982bdbb6f01157c394) style(ui): refine explorer hierarchy
- [`8c6c266`](https://github.com/yelog/lazydb/commit/
8c6c266f75b07c427f2545d42c450d093dd90008) fix(profiles): stabilize visible object selection
- [`ffdcc1f`](https://github.com/yelog/lazydb/commit/
ffdcc1f564b3378f2819d218bc332332d5d5fcf2) feat(profiles): discover visible objects automatically
- [`afc366e`](https://github.com/yelog/lazydb/commit/
afc366e8cc0ed5fdf922551ceac3c8c1f170850c) feat(ui): add configurable terminal icons
- [`a192121`](https://github.com/yelog/lazydb/commit/
a192121a6e290412d9aef17cc7a6f04b8e933983) fix(ui): align workspace tabs with main content
- [`a03daeb`](https://github.com/yelog/lazydb/commit/
a03daeb7bfd9c89fb927ccd6ff16cfe41b3fb06d) chore: ignore codegraph index
- [`92c24fb`](https://github.com/yelog/lazydb/commit/
92c24fb6ceb3a74427c04b1d4a0f2a2389347b76) fix(keymap): preserve leader shortcuts on relation tabs
- [`9b7a32a`](https://github.com/yelog/lazydb/commit/
9b7a32a3f01b279a6e3ba91712cc47091bbf639d) fix(explorer): respect focus for relation shortcuts
- [`813a764`](https://github.com/yelog/lazydb/commit/
813a76419728a9872f13d8bfc9bd5c2312e7d1b9) fix(profiles): persist new connection passwords by default
- [`fb674d7`](https://github.com/yelog/lazydb/commit/
fb674d7d33088a4b3dde16e0b99c63264c854bb7) merge: integrate relation preview controls
- [`4c26bfa`](https://github.com/yelog/lazydb/commit/
4c26bfa89eadef04381230e5b001881fd89e82e5) feat(relation): improve data preview controls
- [`d5207be`](https://github.com/yelog/lazydb/commit/
d5207befb98476448b957d3d1d6b5d5f3facb347) feat(editor): add execution target selector
- [`3adb069`](https://github.com/yelog/lazydb/commit/
3adb069a0a8ae72379358de5b5b789c8f1107c26) feat(profiles): improve connection management
- [`33c6928`](https://github.com/yelog/lazydb/commit/
33c6928dba64250713b6230acf3bcb4e36fb0664) merge: integrate sqleditor worktree into main
- [`942e729`](https://github.com/yelog/lazydb/commit/
942e7296d4dbe42abb8926ba94293ebe090916cc) feat(explorer): complete database explorer and relation pages
- [`0c3513e`](https://github.com/yelog/lazydb/commit/
0c3513e6d98250ed4148238f0dd1d8b9178d8088) feat(ui): move editor context into title
- [`4205592`](https://github.com/yelog/lazydb/commit/
42055923a4cd67227681c7be95d7c1f3a5396813) docs(ui): plan editor context title
- [`cc1270a`](https://github.com/yelog/lazydb/commit/
cc1270a2a286b71bae60d32f073d5c7c04749daf) docs(ui): design editor context title
- [`4d8b1d4`](https://github.com/yelog/lazydb/commit/
4d8b1d4addcbc2251595c5b19eabbf9caef684e2) fix(completion): preserve accepted cursor lifecycle
- [`3ad92cf`](https://github.com/yelog/lazydb/commit/
3ad92cfa714b6bcddea37565bcaaf3a0aa66179a) docs(completion): plan accept cursor lifecycle
- [`a2eece1`](https://github.com/yelog/lazydb/commit/
a2eece1affaa0ec483ef0de7d3bf7fda06eb7b3b) docs(completion): design accept cursor lifecycle
- [`a1105eb`](https://github.com/yelog/lazydb/commit/
a1105ebea17947b12ffec3dad932957303cc38e1) fix(editor): preserve global keys and completion boundaries
- [`a674ad7`](https://github.com/yelog/lazydb/commit/
a674ad7df18bd1739b0b3ae2162cee1126becfff) docs(editor): plan keymap completion lifecycle
- [`5253eeb`](https://github.com/yelog/lazydb/commit/
5253eeb1b2945e5ad82488f8442afbdb24483317) docs(editor): design keymap completion lifecycle
- [`aef8bba`](https://github.com/yelog/lazydb/commit/
aef8bba7565327b22c7ed14ec20ae2e012e9ad40) feat(workspace): debounce saves and configure schema
- [`e6c3040`](https://github.com/yelog/lazydb/commit/
e6c3040d309295840be97ed3f0fbb06d0fab8cf9) feat(sql): add connection target selector
- [`4f598de`](https://github.com/yelog/lazydb/commit/
4f598de8fe2a9c1b009ca3d4d501e51eb3950561) fix(workspace): add single-writer lock and durable saves
- [`6506aef`](https://github.com/yelog/lazydb/commit/
6506aef0c6cc45ba2aff93829f9d49ef2fe777b9) feat(workspace): load restored consoles at startup
- [`64ede18`](https://github.com/yelog/lazydb/commit/
64ede183519c6d916a3121d965c587672440e2d4) fix(workspace): keep targets aligned after profile changes
- [`cab487f`](https://github.com/yelog/lazydb/commit/
cab487f2befa0efe5819a487c8086663463c49df) feat(workspace): persist sql editor snapshots
- [`9f5e34b`](https://github.com/yelog/lazydb/commit/
9f5e34b969482f8ef124d71bb8a6ba3446970cd9) feat(sql): switch connections with editor tabs
- [`8cdf5ce`](https://github.com/yelog/lazydb/commit/
8cdf5ce0f7ccd35062f46e1f222d6867557253a1) feat(sql): add target selection commands
- [`fd5048f`](https://github.com/yelog/lazydb/commit/
fd5048f9baca409a0ce2ee558012f024a4cea023) feat(sql): add editor execution targets
- [`bb3bcd0`](https://github.com/yelog/lazydb/commit/
bb3bcd0c4f1a9a888559010fae5fe583cdc05564) feat(transaction): expose editor controls
- [`6e3f47d`](https://github.com/yelog/lazydb/commit/
6e3f47d96ce9767e3ed34a1f5de1d6d037b6314f) fix(transaction): own cancellation and shutdown cleanup
- [`8b5b1ef`](https://github.com/yelog/lazydb/commit/
8b5b1ef20e42b1c7edc5c20eb3457604f9105645) fix(transaction): reject stale worker commands
- [`2c6df8a`](https://github.com/yelog/lazydb/commit/
2c6df8ad0b511c32d180aeb9dbd6d4af60a4a72a) fix(transaction): await begin and retire workers
- [`36f925a`](https://github.com/yelog/lazydb/commit/
36f925adb42bd602c323c3b58a3c367c409b1562) fix(transaction): honor exit choices and scoped sql
- [`c208316`](https://github.com/yelog/lazydb/commit/
c2083166ec320bfcd7819864b3c0eef15e1dca9f) feat(editor): add cursor styles and selection rendering
- [`7e086d2`](https://github.com/yelog/lazydb/commit/
7e086d2589b39fb5736ca496ec7f3272b831f1dc) fix(editor): route vim input through console machines
- [`cb155e9`](https://github.com/yelog/lazydb/commit/
cb155e9864439e783be778dc1be898e2b7e37f5d) test(editor): characterize modal input failures
- [`044e201`](https://github.com/yelog/lazydb/commit/
044e2019f7ab13fd86b26719bcc8d008f4c6a8fd) feat(explorer): implement database explorer
- [`a7d3ebb`](https://github.com/yelog/lazydb/commit/
a7d3ebbded4516be5232cbc9a1f7b628a133d0af) docs(sql): plan editor runtime context
- [`a4cfd0d`](https://github.com/yelog/lazydb/commit/
a4cfd0d7439cacc579df8c94431677dd6ee77b0e) docs(sql): design editor runtime context
- [`ee3270e`](https://github.com/yelog/lazydb/commit/
ee3270ee81fcb160ce771bbbbd8bb2c99eb29f74) feat(sql): add editor transactions and completion
- [`713ed1e`](https://github.com/yelog/lazydb/commit/
713ed1ed0d98f92713c8124067cfcca107295547) docs: document dynamic connection profiles
- [`d720c2e`](https://github.com/yelog/lazydb/commit/
d720c2e441e29e75eaa624211c5fe9acb1ab1694) test(profiles): cover full lifecycle
- [`b71bd7d`](https://github.com/yelog/lazydb/commit/
b71bd7d1fd636a7544e8adf078e1d2c5788f88ec) feat(profiles): open manager on first launch
- [`ccdc313`](https://github.com/yelog/lazydb/commit/
ccdc31301d50750e57462b7ca75eb49061d735a2) feat(ui): render connection profile manager
- [`aaab930`](https://github.com/yelog/lazydb/commit/
aaab9300ab1723564d6e7c029f226341ff28e116) feat(profiles): add manager input controls
- [`1c32c4d`](https://github.com/yelog/lazydb/commit/
1c32c4db633af15b2a1b1ede503a790656f5b39d) fix(runtime): bind commands to active connections
- [`4fc1ab1`](https://github.com/yelog/lazydb/commit/
4fc1ab11079df71a7b381a7fa77fbcca630c893b) feat(profiles): persist runtime profile changes
- [`c37cf1f`](https://github.com/yelog/lazydb/commit/
c37cf1f1e1f312b8caca6154bd71c7db921241f3) feat(profiles): add profile manager reducer
- [`cdce725`](https://github.com/yelog/lazydb/commit/
cdce725827873e63d9ce030f0385faa7884f8f2d) feat(profiles): add connection draft validation
- [`bb5e9e4`](https://github.com/yelog/lazydb/commit/
bb5e9e47811c30d10670fb90b1917f0909d7a94c) feat(security): add native secret store boundary
- [`20bd92c`](https://github.com/yelog/lazydb/commit/
20bd92ca86d306a216d11603d817d0e31639c4c2) docs: plan dynamic profile manager implementation
- [`b5de003`](https://github.com/yelog/lazydb/commit/
b5de0034e04af05e67b247fb9c1314da2e776a08) docs: design SQL editor and transactions
- [`7b54b9c`](https://github.com/yelog/lazydb/commit/
7b54b9cfdc608554e8e6023724c5888f69b5ecf9) docs: design dynamic profile manager
- [`074322d`](https://github.com/yelog/lazydb/commit/
074322d4bfe3b126a98471ee89347b7453dd622a) feat: implement LazyDB M0 foundation

## [0.1.0-beta.2] - 2026-09-01

### Added

- Added project-scoped connection access and per-connection workspace persistence.
- Added paginated database results, contextual SQL completion, data-grid navigation, contextual key hints, and persistent shortcut help.
- Added a notification center, motion-aware loading feedback, and improved relation editing and data hierarchy controls.
- Added headless coding-agent database access through CLI and project-scoped MCP tools, with explicit read/write policy boundaries.
- Added native installation and update channels with standalone installer pages and release manifests.

### Changed

- Improved keyboard navigation, pane resizing, workspace tab controls, explorer interaction, and visible-object loading feedback.
- Refined PostgreSQL catalog search and qualified completion behavior across schemas, relations, and columns.
- Extracted the Neovim integration into its standalone repository and clarified installation and onboarding documentation.

### Fixed

- Fixed relation mutation stability, catalog readiness during tab restoration, SQLite mutation compatibility, SQL execution error focus, null rendering, key sequence handling, and result-filter layout behavior.
- Fixed grid scrolling, selection visibility, edited-cell highlighting, relation shortcuts, explorer search separators, installer portability, and CI integration-test stability.

### Security

- Added project-scoped database access controls and explicit coding-agent database write policy enforcement.

### Internal

- Updated CI dependencies and runner configuration, expanded database and UI integration coverage, and serialized database integration tests where required.

### Commits
- [`d34fd20`](https://github.com/yelog/lazydb/commit/d34fd20ae31feb2fb27357022d9d34a33bae38f2) test(postgres): avoid assuming search returns relation children
- [`93b6970`](https://github.com/yelog/lazydb/commit/93b6970a54f7bed17ff43dd6efdb576267fd6d21) test(postgres): assert column metadata separately
- [`4de41a0`](https://github.com/yelog/lazydb/commit/4de41a0ea3c41299ed28a885c34bd606908b53e8) test(postgres): use valid bounded column search
- [`e220f26`](https://github.com/yelog/lazydb/commit/e220f265ece56613e543aaf125b0ff15a40e54ac) test(postgres): avoid truncating column search results
- [`aa755f1`](https://github.com/yelog/lazydb/commit/aa755f1d1abdcb906e2740e98670393591152488) test(postgres): allow room for relation search children
- [`4e284ec`](https://github.com/yelog/lazydb/commit/4e284ec6333bd67a0c4afb8d99c713ce2d1bd6a0) test(postgres): search table columns through relation match
- [`1b485e9`](https://github.com/yelog/lazydb/commit/1b485e9c5c8d308b617208a448b0c2a537c637e3) test(postgres): search columns by scoped name
- [`2f957d9`](https://github.com/yelog/lazydb/commit/2f957d9ea5f3312feeb43a76c43e722f65f1bcb0) test(postgres): match columns by relation suffix
- [`02b3411`](https://github.com/yelog/lazydb/commit/02b34114230dcc59ced0c30f229b74bd59743924) test(postgres): search columns by full catalog path
- [`8962c3e`](https://github.com/yelog/lazydb/commit/8962c3e7891062fe90096dad496bbe124d47f64b) ci: serialize database integration tests
- [`fa11787`](https://github.com/yelog/lazydb/commit/fa117871acb55e268c0f37386a57654409d4060e) fix(ci): resolve clippy and mysql test failures
- [`73e5417`](https://github.com/yelog/lazydb/commit/73e54179947ba759bb9143ae4f2f2b2f68af029b) fix(explorer): ignore separators in search
- [`d8100e6`](https://github.com/yelog/lazydb/commit/d8100e6a29f59f81a17c063658f24097dd7432ca) fix(keymap): enable leader shortcuts outside explorer
- [`5d8d557`](https://github.com/yelog/lazydb/commit/5d8d55785f95dadd74a75cf88423b0343cc8669a) fix(ui): distinguish null values in data views
- [`d34a03a`](https://github.com/yelog/lazydb/commit/d34a03a9715fedbad2d89fd8a4967a76f030b848) fix(sql): focus output when execution fails
- [`385638c`](https://github.com/yelog/lazydb/commit/385638cbf1521436023cc583f88882bd81b53e57) fix(keymap): preserve relation shortcuts
- [`0db886c`](https://github.com/yelog/lazydb/commit/0db886ce4cc64e5de70ed0fa62293a53305c71fb) merge: add notification center
- [`0bb2a1a`](https://github.com/yelog/lazydb/commit/0bb2a1a8b98e2b9f3fc4f14e77cd277e280e1b63) docs: add notification center design
- [`a3d027b`](https://github.com/yelog/lazydb/commit/a3d027b2ce392c970a67711d22569ca4c5909ff4) feat(ui): add notification center
- [`bf73ab8`](https://github.com/yelog/lazydb/commit/bf73ab84631fb1385f712e820741d5d4dce374cb) test(ui): focus transaction help assertion
- [`8b7fc54`](https://github.com/yelog/lazydb/commit/8b7fc548c76a7f07a3260aac6ec157981eb731d6) feat(ui): add persistent shortcut popup
- [`b289109`](https://github.com/yelog/lazydb/commit/b289109815cd0d9470f25e2333673a44172a1811) feat(ui): improve workspace tab controls
- [`cb69915`](https://github.com/yelog/lazydb/commit/cb69915ed1e35e2559249a193d3cf73bc835e206) fix(ui): show relation commit success
- [`2e35ebd`](https://github.com/yelog/lazydb/commit/2e35ebd1f5397252fb38c702612809164b6de138) fix(postgres): stabilize relation mutations
- [`163f754`](https://github.com/yelog/lazydb/commit/163f7547fd8ca740b71d1a30cc4b7996663f040d) fix(ui): highlight only edited relation cells
- [`41ab957`](https://github.com/yelog/lazydb/commit/41ab9573c49e86be151af58fa942db88f8163f9e) fix(pages): make installers standalone
- [`9adb3ef`](https://github.com/yelog/lazydb/commit/9adb3ef4b50f80b6b0a466aaf2c77346cf7406ca) docs: correct keyboard shortcut reference
- [`55ba036`](https://github.com/yelog/lazydb/commit/55ba036f013e5df05f03e58083cec42618c79af3) merge: add contextual keyboard hints
- [`575d98e`](https://github.com/yelog/lazydb/commit/575d98ef97eb9f2a26e055401e72c2d7f0a66a02) feat(ui): add contextual keyboard hints
- [`f0fccd3`](https://github.com/yelog/lazydb/commit/f0fccd38b41957126c7f9d9fc1414a2a4acc9e96) fix(sqlite): avoid unsupported returning syntax
- [`f465259`](https://github.com/yelog/lazydb/commit/f46525946c7b98103f73bec50267fbe1db7861fc) merge: add safe Explorer catalog drops
- [`41b3330`](https://github.com/yelog/lazydb/commit/41b3330ad626f038969de2fcb5e123a0b80c2212) feat(explorer): safely drop catalog objects
- [`caf95a6`](https://github.com/yelog/lazydb/commit/caf95a6d30b3ea9274cb0e120363c95bc76904cc) fix: stabilize relation table editing
- [`afb78a4`](https://github.com/yelog/lazydb/commit/afb78a45954f2506efb63d0a1982850f8c8c8ffc) fix: preserve pagination key bindings after merge
- [`d047495`](https://github.com/yelog/lazydb/commit/d0474952d2428b24b81d962a2750b63f2c66bdc7) merge: add paginated database results
- [`a60e7c7`](https://github.com/yelog/lazydb/commit/a60e7c7948bf08aeaad4debeece0b857413b24d4) feat: add paginated database results
- [`86d06cc`](https://github.com/yelog/lazydb/commit/86d06cc3c774155bc1d7f4bffc6ccb6d42fc7899) docs: add implementation plans
- [`18afd88`](https://github.com/yelog/lazydb/commit/18afd88460c8a3a559e135a737767eb52e29b60e) feat(ui): navigate completion candidates with arrows
- [`be519ef`](https://github.com/yelog/lazydb/commit/be519efa8edb42e375ee13163ef7065bf85da4be) merge: add context-aware SQL completion
- [`1f797e4`](https://github.com/yelog/lazydb/commit/1f797e47014518c6cab508c0c7a41d933b1b779a) feat(sql): make completion context-aware
- [`1d13745`](https://github.com/yelog/lazydb/commit/1d13745ee9cc96937aa2d67f9559faa231ec6643) fix(ui): distinguish data grid header from selection
- [`556809c`](https://github.com/yelog/lazydb/commit/556809c106de607e83d1693005274282898079c1) feat(ui): alias zero to first data grid column
- [`c135f54`](https://github.com/yelog/lazydb/commit/c135f54636a7383f329cf8cff9380577fdf28586) feat(ui): add data grid column jump shortcuts
- [`9489da3`](https://github.com/yelog/lazydb/commit/9489da361153f40b66a1aafc0b8daa63cc009089) fix(ui): lower result filter layout breakpoint
- [`40a278b`](https://github.com/yelog/lazydb/commit/40a278bd984d4cee833e2ebe971619102b06354c) fix(ui): place result filters inside panel
- [`76dc6fb`](https://github.com/yelog/lazydb/commit/76dc6fb3a29d03b09027ce454fe7cda4475567ae) test(ui): restore result completion coverage
- [`e984996`](https://github.com/yelog/lazydb/commit/e984996218dc2869916438d8eb6579c052fd7dec) merge: clarify SQL result filter lifecycle
- [`fb54bb6`](https://github.com/yelog/lazydb/commit/fb54bb654830576330563618b6c192246914dfce) fix(sql): clarify result filter lifecycle
- [`5d360e6`](https://github.com/yelog/lazydb/commit/5d360e66481a3a8805718f4041fca420a0dcf93a) merge: refine relation data hierarchy
- [`54b199e`](https://github.com/yelog/lazydb/commit/54b199e4db91732bf35b9f08b569cc68014a09df) feat(ui): refine relation data hierarchy
- [`3f41f6d`](https://github.com/yelog/lazydb/commit/3f41f6d2edbce4cbb7da4c44072edb745e830305) fix(ui): stabilize explorer expansion rendering
- [`a7fcfd7`](https://github.com/yelog/lazydb/commit/a7fcfd7eb55b520432762e9a192cc8cbd215c2b9) docs: add relation data visual hierarchy plans
- [`18596be`](https://github.com/yelog/lazydb/commit/18596be305894692917cd90ed2aa5ffb0d382dca) merge: add native install and update channels
- [`5d1a374`](https://github.com/yelog/lazydb/commit/5d1a37488dabdde5c3343a554e3a1cf9bdfcd526) feat(release): add native install and update channels
- [`dee9eb5`](https://github.com/yelog/lazydb/commit/dee9eb5e29d4d5326a4d8173dee17334f6752e9e) Merge branch 'task/restored-relation-catalog-readiness'
- [`7b0a804`](https://github.com/yelog/lazydb/commit/7b0a804115c36c585df427ad9b1ee8e5262c7964) fix(relation): wait for catalog before restoring tabs
- [`f049aad`](https://github.com/yelog/lazydb/commit/f049aad7845c30703bda88dd5dfea0005a4aed3e) fix(ci): align UI and catalog test contracts
- [`3271696`](https://github.com/yelog/lazydb/commit/3271696621ef68117673aff44f90d9a4c8571dd2) fix(ci): stabilize full codebase integration
- [`10a314c`](https://github.com/yelog/lazydb/commit/10a314c517d832c4a70af44640a3b6e4ebeb838b) feat(ui): complete interaction and help updates
- [`dbd607e`](https://github.com/yelog/lazydb/commit/dbd607ec9ac1a750fd25e1beac62ad785bb94507) merge: improve visible object picker feedback
- [`0d90811`](https://github.com/yelog/lazydb/commit/0d9081145d21ad7c88c7e15e076bf72c7b8f14b8) feat(ui): improve visible object picker feedback
- [`406a167`](https://github.com/yelog/lazydb/commit/406a167bf14f763d63ad4b2dde275e02eb814f2a) feat(ui): add help and viewport interaction updates
- [`3c47b2e`](https://github.com/yelog/lazydb/commit/3c47b2e9714a9e9939247c1e7d8341ade8575dbe) fix(mouse): scroll grid and explorer viewports immediately
- [`ec8abad`](https://github.com/yelog/lazydb/commit/ec8abadf31ecc56e930e4849e20c0297562adb05) fix(postgres): match structured catalog child paths
- [`c62a472`](https://github.com/yelog/lazydb/commit/c62a47280d7708b2f01d3f2b471c9ae7ac9823b1) fix: document alternate help shortcut
- [`88ba8c3`](https://github.com/yelog/lazydb/commit/88ba8c325cc2f4efb673621b7ee9c48569ace52a) fix(postgres): match full catalog search paths
- [`0af4738`](https://github.com/yelog/lazydb/commit/0af47384ae1472fb3962cd718e9ead0756329f3e) fix(postgres): prioritize qualified object suffixes
- [`b5deee1`](https://github.com/yelog/lazydb/commit/b5deee1c25ff13e6f64473eb7686f863162d89c7) docs: design contextual help shortcut
- [`e86925c`](https://github.com/yelog/lazydb/commit/e86925c6abb0207b30389d0b1a9e753b21589582) fix(postgres): prioritize qualified catalog matches
- [`c55daff`](https://github.com/yelog/lazydb/commit/c55daffabe7f254a9b4a33542628b4ced195cda3) fix(postgres): keep catalog search path scoped
- [`e4b7752`](https://github.com/yelog/lazydb/commit/e4b7752be138ed7202b4fe4ef27c6c11b37e24c6) fix(postgres): match qualified catalog searches
- [`8cdb6b6`](https://github.com/yelog/lazydb/commit/8cdb6b6f700fdfede7d93d61362225d514d104fd) fix(postgres): tolerate catalog output variations
- [`a8614d2`](https://github.com/yelog/lazydb/commit/a8614d26edab79134582fa5362c0ee768027936f) docs: restructure README for user onboarding
- [`39bc211`](https://github.com/yelog/lazydb/commit/39bc211ed5bd923379b219f49024c069c94e99b5) fix(postgres): exclude trigger functions from catalog
- [`065d69f`](https://github.com/yelog/lazydb/commit/065d69fd3edb6853db29bc31ca3941180c5aeac5) docs: add coding agent database access plan
- [`250298e`](https://github.com/yelog/lazydb/commit/250298e638f29b6a19760843e49bd5d8c0fe1fd5) merge: add coding agent database access
- [`58633ee`](https://github.com/yelog/lazydb/commit/58633ee20b2f4ccd77d724cdaea76a3ee0138f4f) fix(ui): highlight SQL in DDL and output logs
- [`9e79cd7`](https://github.com/yelog/lazydb/commit/9e79cd73c08e1d3c13a1f29312e179487fc296f0) fix(input): refresh pending key sequence timeout
- [`85fd406`](https://github.com/yelog/lazydb/commit/85fd406c11199cd40802aaca9befcbbc3ee14809) test(agent): harden database access boundaries
- [`4978187`](https://github.com/yelog/lazydb/commit/4978187002b6961dae31a15c84dd7478879dd94e) docs: explain coding agent database access
- [`204cd55`](https://github.com/yelog/lazydb/commit/204cd55e138fd8dd297e698254fa1ea08ee76b3d) feat(mcp): serve project-scoped database tools
- [`d824f6e`](https://github.com/yelog/lazydb/commit/d824f6ef927b2d46d9f21d07002006ac90a10375) feat(cli): add coding agent database commands
- [`1b01284`](https://github.com/yelog/lazydb/commit/1b01284b012f0732e33e68d7daa3edd8128ef73d) feat(agent): expose progressive schema inspection
- [`48c03f8`](https://github.com/yelog/lazydb/commit/48c03f81731e4f07075e649e41e8975cf9a8ee51) feat(agent): add headless database service
- [`f80e684`](https://github.com/yelog/lazydb/commit/f80e684ed3d4620eefd8ef4a8be4a59c2cbf683f) feat(agent): define API and write policy
- [`3f97180`](https://github.com/yelog/lazydb/commit/3f971801433782f2ad8131c4fc4dd69fd12f2259) merge: add motion-aware UI feedback
- [`d51c406`](https://github.com/yelog/lazydb/commit/d51c406bb2e0e6691274892ca0b7c542a81f8633) feat(ui): add motion-aware loading feedback
- [`a1599a6`](https://github.com/yelog/lazydb/commit/a1599a66818b8d2b81f4fc4604b2707d3b0a46d8) refactor(credentials): share headless profile resolution
- [`5842036`](https://github.com/yelog/lazydb/commit/5842036c9a6a55d79d1fdbfe06255a22adfc9b29) feat(agent): select connections deterministically
- [`396c0d9`](https://github.com/yelog/lazydb/commit/396c0d9e46179b2db926dc801b603a790483cb60) feat(agent): resolve project-visible connections
- [`5678d27`](https://github.com/yelog/lazydb/commit/5678d279a534b63e9540cdb28713186afe4e8f05) merge: add vim-style pane resizing
- [`f9bb215`](https://github.com/yelog/lazydb/commit/f9bb2152b2b712b62764f06f28cbd157ecacfee5) feat(layout): add vim-style pane resizing
- [`f69118c`](https://github.com/yelog/lazydb/commit/f69118c25604d6ef557aa7d84243fe67cb327e49) docs: simplify binary installation instructions
- [`de110f9`](https://github.com/yelog/lazydb/commit/de110f975883ae57ea1563d8435127c327d15904) merge: stabilize SQL completion popup position
- [`de3d7a7`](https://github.com/yelog/lazydb/commit/de3d7a79d821d37734a68600973e84069534c316) fix(ui): stabilize SQL completion popup position
- [`5273250`](https://github.com/yelog/lazydb/commit/527325035170c82c926e09501dc14d7c76620e79) docs: add implementation plans
- [`56c78c9`](https://github.com/yelog/lazydb/commit/56c78c9d63b0bd42689ab584c8de53245cb2d83e) feat: add read-only vim copy views
- [`34f8271`](https://github.com/yelog/lazydb/commit/34f8271cbf6bb06306aec4b3bfa28689b355f9a0) refactor(neovim): move plugin to standalone repository
- [`be20242`](https://github.com/yelog/lazydb/commit/be20242c77a4b48897af98d45f60fe848fbb417e) docs(neovim): update extraction execution plan
- [`87fb8b5`](https://github.com/yelog/lazydb/commit/87fb8b5a98b72d280c01e4317d42e4098c0e9e17) docs(neovim): preserve filtered extraction history
- [`6cf7b18`](https://github.com/yelog/lazydb/commit/6cf7b183777fa7f3eceab371830dddee25589197) docs(neovim): design standalone plugin extraction
- [`e790999`](https://github.com/yelog/lazydb/commit/e790999d18515042f3e0ca187419372b1c82c87b) fix(record-view): highlight selected field
- [`1a3f37f`](https://github.com/yelog/lazydb/commit/1a3f37f4b94fc3f9bacc78da11a1fc4b9d79c725) merge: integrate project-scoped connections
- [`b84fba9`](https://github.com/yelog/lazydb/commit/b84fba96f3deebea9d27debf8fcefa42457842ea) docs(neovim): document current repository installation
- [`c91da18`](https://github.com/yelog/lazydb/commit/c91da182b4f4a4d2b0eb14fae509250cdcabdb47) fix(clipboard): handle shifted row copy keys
- [`555a2fa`](https://github.com/yelog/lazydb/commit/555a2fa0ebc8f36bb256ce84f684c4709916951c) feat: show active connection in others group
- [`7384eb9`](https://github.com/yelog/lazydb/commit/7384eb9202bf894795d472bd75fac1d3f06039dd) docs: explain project-scoped connections
- [`a56ca2d`](https://github.com/yelog/lazydb/commit/a56ca2d95a73974de2749fdf3a8a8b3e72ce2e37) feat: reveal scoped startup connections
- [`5c4f9b3`](https://github.com/yelog/lazydb/commit/5c4f9b3aa50e890ac46bc39122a205f61252290d) feat: render project-aware connections
- [`6146471`](https://github.com/yelog/lazydb/commit/6146471f3679e0c2e381469e8f2e8a0f8c76a9e2) feat: add connection access menu
- [`466ae3d`](https://github.com/yelog/lazydb/commit/466ae3d661aa2d92a256069c72052cdfbeac4ba9) feat: update connection access transactionally
- [`ef08634`](https://github.com/yelog/lazydb/commit/ef08634bfa6ce4e9ffe9ccb52b6e6170664cb277) feat: scope new connections to current project
- [`6dd733e`](https://github.com/yelog/lazydb/commit/6dd733e16c1e3eced12695500280805d26fbe68b) feat: group unrelated connections in explorer
- [`5b926e1`](https://github.com/yelog/lazydb/commit/5b926e17decc85e1dd3887b1e0a17453447d9971) feat: pass project context into app
- [`8e582a9`](https://github.com/yelog/lazydb/commit/8e582a95b2ddf2d2af076dfee2a548d4e7cd0d9e) feat: persist connection access scope
- [`1d8b1b5`](https://github.com/yelog/lazydb/commit/1d8b1b5c7dc5b3c2f0ae119cf4a4d1242e3e4208) feat: resolve current project context
- [`6b31778`](https://github.com/yelog/lazydb/commit/6b3177886c8e6bc46f6b1d0db4e49b717d4f3d2e) merge: integrate trailing partial grid column
- [`0f93d2b`](https://github.com/yelog/lazydb/commit/0f93d2b59c8afde095e5590780c0b0274c15f0c1) merge: integrate context-aware copy
- [`49742a8`](https://github.com/yelog/lazydb/commit/49742a890ffba22f5614cd5a091ab017cca52ddf) feat: add per-connection workspaces
- [`52be7a2`](https://github.com/yelog/lazydb/commit/52be7a2e49b9d6aa2e49e72ea86760f680418d80) feat(clipboard): add context-aware copy actions
- [`778337e`](https://github.com/yelog/lazydb/commit/778337eebf89d15ee7f4384d79e209854689ee5c) Merge branch 'task/keyboard-navigation'
- [`29bd791`](https://github.com/yelog/lazydb/commit/29bd79107de5660fd5a4505f70cda36682463b5d) fix(ui): render trailing partial grid column
- [`a870c51`](https://github.com/yelog/lazydb/commit/a870c5106194ead0deed49c80cce54d8bea5194d) Merge branch 'main' of github.com:yelog/lazydb
- [`1d56695`](https://github.com/yelog/lazydb/commit/1d56695f890e5719a91d3691b7de27e509abe692) chore(deps): bump actions/checkout to v7.0.1
- [`6c3d4d0`](https://github.com/yelog/lazydb/commit/6c3d4d02334bce22109cf2c0b6f457a0a30116fa) chore(deps): bump actions/upload-artifact to v7.0.1
- [`cd44e2c`](https://github.com/yelog/lazydb/commit/cd44e2c2111922c83dc59d7d302f12b19c5994d7) chore(deps): bump actions/download-artifact to v8.0.1
- [`39a550b`](https://github.com/yelog/lazydb/commit/39a550b9e2a79fe68229b7e3ae870439026a6eca) chore(deps): bump actions/attest-build-provenance to v4.2.2
- [`970410e`](https://github.com/yelog/lazydb/commit/970410eb3066ebcb16d5750457039466b54c071e) chore(deps): bump actions/cache to v6.1.0
- [`fcdd45d`](https://github.com/yelog/lazydb/commit/fcdd45d41f6b7b87cc2180e949a4d820d6c1886a) ci(release): use current Intel macOS runner
- [`0407625`](https://github.com/yelog/lazydb/commit/0407625714ab1696039552bdd4cfcfb9260b5291) fix(workspace): address per-connection regressions
- [`c470a4e`](https://github.com/yelog/lazydb/commit/c470a4ed1063d405abb1f71f423d74e98cd18614) docs(workspace): explain per-connection tab restoration
- [`ff2592d`](https://github.com/yelog/lazydb/commit/ff2592d61f0be48bfefab5c3c26b349933268d5e) feat(workspace): delete workspace with its profile
- [`49fcb9b`](https://github.com/yelog/lazydb/commit/49fcb9b22d0030d4e6bd9b6fd90aeca74f154349) fix(workspace): scope console lifecycle by profile
- [`ed79fdd`](https://github.com/yelog/lazydb/commit/ed79fdd3f78a4be050077e41db12c4b3d18137df) feat(workspace): restore relation tabs lazily
- [`c16b76c`](https://github.com/yelog/lazydb/commit/c16b76c6eb595330f701c86428b28b33549958ff) feat(workspace): hide tabs when a profile disconnects
- [`0fe15e0`](https://github.com/yelog/lazydb/commit/0fe15e05494e5682825cafb3435eddbe6c09c600) fix(workspace): guard connection switches against live work
- [`3a143cc`](https://github.com/yelog/lazydb/commit/3a143cc8c6d79340a0183c6a12771846a1117059) feat(workspace): swap tabs after successful connection
- [`db8f7c3`](https://github.com/yelog/lazydb/commit/db8f7c3cac2d0e8f67e25d18de96684a20cfd6cd) feat(workspace): snapshot and restore every profile workspace
- [`040540f`](https://github.com/yelog/lazydb/commit/040540fedf561a55a6ec65d1bbf18b6127379015) feat(workspace): migrate flat workspaces by profile
- [`81cf7bc`](https://github.com/yelog/lazydb/commit/81cf7bcda8fa42ff806b2328f20a20f555d00759) feat(workspace): define profile-scoped persistence format
- [`718648d`](https://github.com/yelog/lazydb/commit/718648d866ee392c4299d11bdfbf318555354daf) feat: improve keyboard navigation
- [`7c5df07`](https://github.com/yelog/lazydb/commit/7c5df07f54ec6b559df52ed1d71ccf6bb3463eff) refactor(workspace): add profile-scoped workspace state
- [`bd699b4`](https://github.com/yelog/lazydb/commit/bd699b409054139cf75f04bc5e9d375acea8be1e) docs: design keyboard navigation

## Unreleased

Changes that are not part of a tagged release go here.

### Added

- Documented per-connection workspaces: profile switches are committed only
  after a successful connection, failed switches preserve the current workspace,
  disconnecting hides rather than deletes a workspace, relation tabs restore as
  lazy shells without persisting result data across restarts, and profile
  deletion removes the workspace.

[unreleased]: https://github.com/yelog/lazydb/compare/HEAD...HEAD
