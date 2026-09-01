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
