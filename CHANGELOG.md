# Changelog

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

## Unreleased

Changes that are not part of a tagged release go here.

[unreleased]: https://github.com/yelog/lazydb/compare/HEAD...HEAD
