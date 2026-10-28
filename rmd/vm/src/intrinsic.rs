use core::types::Identifier;

use crate::{FaultKind, GenericValue, Intrinsic, eval::Evaluator, value::Src};

mod byond;
mod demir;
mod view_grid;

use byond::Result;

impl Evaluator<'_> {
    pub(crate) fn intrinsic_impl(
        &mut self, intrinsic: Intrinsic, src: Src, name: &str, params: &[Identifier],
        args: Vec<(Option<Identifier>, GenericValue)>,
    ) -> Result<GenericValue> {
        let arg = |n: usize| args.get(n).map(|(_, value)| value.clone()).unwrap_or_default();
        let text = |n: usize| args.get(n).and_then(|(_, value)| value.text()).unwrap_or_default();
        let positional = || args.iter().map(|(_, value)| value.clone()).collect::<Vec<_>>();

        match intrinsic {
            Intrinsic::Call | Intrinsic::CallExt => self.dm_call(&positional()),
            Intrinsic::Arglist => self.arglist(arg(0)),

            Intrinsic::WorldNew | Intrinsic::WorldDel | Intrinsic::WorldTopic | Intrinsic::WorldError => {
                Ok(GenericValue::Null)
            },

            Intrinsic::WorldIsSubscribed
            | Intrinsic::WorldGetScores
            | Intrinsic::WorldGetMedal
            | Intrinsic::WorldGetCredits => Ok(GenericValue::Null),
            Intrinsic::WorldIsBanned => Ok(false.into()),
            Intrinsic::WorldAddCredits | Intrinsic::WorldPayCredits => Ok(0.into()),
            Intrinsic::WorldSetScores | Intrinsic::WorldSetMedal | Intrinsic::WorldClearMedal => Ok(GenericValue::Null),
            Intrinsic::WorldProfile | Intrinsic::WorldGetConfig | Intrinsic::WorldSetConfig => Ok(GenericValue::Null),

            Intrinsic::WorldReboot
            | Intrinsic::WorldRepop
            | Intrinsic::WorldExport
            | Intrinsic::WorldImport
            | Intrinsic::WorldOpenPort => Err(self.fault(FaultKind::Blocked(name.into()))),

            Intrinsic::WorldFile2List => self.file2list(&arg(0), &arg(1)),

            Intrinsic::DatumNew
            | Intrinsic::DatumDel
            | Intrinsic::DatumTopic
            | Intrinsic::AtomClick
            | Intrinsic::AtomDblClick
            | Intrinsic::AtomMouseDown
            | Intrinsic::AtomMouseDrag
            | Intrinsic::AtomMouseDrop
            | Intrinsic::AtomMouseEntered
            | Intrinsic::AtomMouseExited
            | Intrinsic::AtomMouseMove
            | Intrinsic::AtomMouseUp
            | Intrinsic::AtomMouseWheel
            | Intrinsic::AtomEntered
            | Intrinsic::AtomExited
            | Intrinsic::AtomCrossed
            | Intrinsic::AtomUncrossed
            | Intrinsic::AtomStat
            | Intrinsic::MovableBump => Ok(GenericValue::Null),

            Intrinsic::DmNewIcon => Ok(arg(0)),
            Intrinsic::Block => self.block(&positional()),
            Intrinsic::Range | Intrinsic::Orange => self.range([arg(0), arg(1)], intrinsic == Intrinsic::Range),
            Intrinsic::View | Intrinsic::Oview => self.view([arg(0), arg(1)], intrinsic == Intrinsic::View),
            Intrinsic::DmIconSize => self.dm_icon_size(arg(0), self.number(&arg(1))?),
            Intrinsic::IconStates => self.icon_states(arg(0)),

            Intrinsic::DatumRead
            | Intrinsic::DatumWrite
            | Intrinsic::MovableMove
            | Intrinsic::MobLogin
            | Intrinsic::MobLogout
            | Intrinsic::ClientNew
            | Intrinsic::ClientDel
            | Intrinsic::ClientTopic
            | Intrinsic::ClientStat
            | Intrinsic::ClientCommand
            | Intrinsic::ClientImport
            | Intrinsic::ClientExport
            | Intrinsic::ClientAllowUpload
            | Intrinsic::ClientSoundQuery
            | Intrinsic::ClientMeasureText
            | Intrinsic::ClientMove
            | Intrinsic::ClientClick
            | Intrinsic::ClientDblClick
            | Intrinsic::ClientMouseDown
            | Intrinsic::ClientMouseDrag
            | Intrinsic::ClientMouseDrop
            | Intrinsic::ClientMouseEntered
            | Intrinsic::ClientMouseExited
            | Intrinsic::ClientMouseMove
            | Intrinsic::ClientMouseUp
            | Intrinsic::ClientMouseWheel
            | Intrinsic::ClientIsByondMember
            | Intrinsic::ClientCheckPassport
            | Intrinsic::ClientSendPage
            | Intrinsic::ClientGetAPI
            | Intrinsic::ClientSetAPI
            | Intrinsic::ClientRenderIcon
            | Intrinsic::SavefileNew
            | Intrinsic::SavefileFlush
            | Intrinsic::SavefileExportText
            | Intrinsic::SavefileImportText
            | Intrinsic::SavefileLock
            | Intrinsic::SavefileUnlock
            | Intrinsic::VectorNew
            | Intrinsic::VectorCross
            | Intrinsic::VectorDot
            | Intrinsic::VectorInterpolate
            | Intrinsic::VectorNormalize
            | Intrinsic::VectorTurn
            | Intrinsic::PixlocNew
            | Intrinsic::DmTurnIcon
            | Intrinsic::DmFlipIcon
            | Intrinsic::DmShiftIcon
            | Intrinsic::DmIconIntensity
            | Intrinsic::DmIconBlend
            | Intrinsic::DmIconSwapColor
            | Intrinsic::DmIconDrawBox
            | Intrinsic::DmIconInsert
            | Intrinsic::DmIconMapColors
            | Intrinsic::DmIconScale
            | Intrinsic::DmIconCrop
            | Intrinsic::DmIconGetPixel
            | Intrinsic::DmDatabase
            | Intrinsic::MakeGenerator => Err(self.fault(FaultKind::Blocked(name.into()))),

            Intrinsic::DemirProfile => self.profile(),

            Intrinsic::DemirNodeGroup => self.node_group(name, arg(0), arg(1), arg(2), arg(3)),

            Intrinsic::DemirNodeOrientation => self.node_orientation(name, &args),

            Intrinsic::DemirRotatable => self.rotatable(name, &args),

            Intrinsic::DemirModularRoot => self.modular_root(name, &args),

            Intrinsic::DemirPlaceModular => self.place_modular(arg(0)),

            Intrinsic::DemirTomlDecode => self.toml_decode(text(0)),

            Intrinsic::DemirDefineGroup => self.define_group(name, &args),

            Intrinsic::DemirMapPath => self.map_path(),

            Intrinsic::DemirRebake
            | Intrinsic::DemirModularPreview
            | Intrinsic::DemirSetModularPreview
            | Intrinsic::ImguiDockspace
            | Intrinsic::ImguiSetNextWindowDock
            | Intrinsic::ImguiSetNextWindowSize
            | Intrinsic::ImguiBegin
            | Intrinsic::ImguiEnd
            | Intrinsic::ImguiText
            | Intrinsic::ImguiTextColored
            | Intrinsic::ImguiButton
            | Intrinsic::ImguiCheckbox
            | Intrinsic::ImguiRadio
            | Intrinsic::ImguiSlider
            | Intrinsic::ImguiDrag
            | Intrinsic::ImguiInputText
            | Intrinsic::ImguiSeparator
            | Intrinsic::ImguiSameLine
            | Intrinsic::ImguiTree
            | Intrinsic::ImguiTreeEnd
            | Intrinsic::ImguiCollapsingHeader
            | Intrinsic::ImguiMousePopup
            | Intrinsic::ImguiOpenPopup
            | Intrinsic::ImguiBeginPopup
            | Intrinsic::ImguiEndPopup => self.imgui(intrinsic, name, args),

            // Both play out over time, and a bake shows the appearance from before either starts
            Intrinsic::Animate | Intrinsic::Flick => Ok(GenericValue::Null),

            Intrinsic::Browse
            | Intrinsic::BrowseRsc
            | Intrinsic::Fcopy
            | Intrinsic::Fdel
            | Intrinsic::Ftp
            | Intrinsic::Link
            | Intrinsic::Shell
            | Intrinsic::Shutdown
            | Intrinsic::Text2file
            | Intrinsic::Winget
            | Intrinsic::Winset => Err(self.fault(FaultKind::Blocked(name.into()))),

            Intrinsic::Sleep => Err(self.fault(FaultKind::Sleep)),

            Intrinsic::Image | Intrinsic::MutableAppearance => self.appearance_object(intrinsic, None, params, args),
            Intrinsic::ImageNew => self.appearance_object(intrinsic, src.object(), params, args),
            Intrinsic::Issaved => Ok(self.is_saved(&arg(0), &arg(1)).into()),

            Intrinsic::FcopyRsc => Ok(arg(0)),

            Intrinsic::ListAdd
            | Intrinsic::ListCopy
            | Intrinsic::ListCut
            | Intrinsic::ListFind
            | Intrinsic::ListInsert
            | Intrinsic::ListJoin
            | Intrinsic::ListRemove
            | Intrinsic::ListRemoveAll
            | Intrinsic::ListSwap
            | Intrinsic::ListSplice => {
                let id = src.list().ok_or_else(|| self.fault(FaultKind::InvalidReference))?;
                self.list_builtin(intrinsic, id, args.into_iter().map(|(_, value)| value).collect())
            },

            Intrinsic::Nameof => self.nameof(arg(0)),
            Intrinsic::File2text => self.file2text(&arg(0)),
            Intrinsic::Fexists => self.fexists(&arg(0)),
            Intrinsic::File => self.file(&arg(0), text(0)),
            Intrinsic::Flist => self.flist(&arg(0)),

            Intrinsic::Isfile => Ok(self.isfile(&arg(0)).into()),
            Intrinsic::Text => self.dm_text(arg(0)),
            Intrinsic::Isnull => Ok(self.isnull(&arg(0)).into()),
            Intrinsic::Isnum => Ok(self.isnum(&arg(0)).into()),
            Intrinsic::Istext => Ok(self.istext(&arg(0)).into()),
            Intrinsic::Islist => Ok(self.islist(&arg(0)).into()),
            Intrinsic::Isicon => Ok(self.isicon(arg(0)).into()),
            Intrinsic::Ispath => Ok(self.ispath(arg(0), arg(1)).into()),
            Intrinsic::Istype => Ok(self.istype(arg(0), arg(1)).into()),
            Intrinsic::Isturf
            | Intrinsic::Isarea
            | Intrinsic::Ismovable
            | Intrinsic::Isobj
            | Intrinsic::Ismob
            | Intrinsic::Isloc => Ok(self.is_atom_kind(intrinsic, &arg(0)).into()),
            Intrinsic::Length | Intrinsic::LengthChar => Ok(self.length(arg(0)).into()),
            Intrinsic::Typesof => self.typesof(positional()),
            Intrinsic::Text2path => Ok(self.text2path(text(0))),
            Intrinsic::Min | Intrinsic::Max => self.min_max(intrinsic, positional()),
            Intrinsic::Abs => self.abs(&arg(0)),
            Intrinsic::Ceil => self.ceil(&arg(0)),
            Intrinsic::Floor => self.floor(&arg(0)),
            Intrinsic::Sqrt => self.sqrt(&arg(0)),
            Intrinsic::Round => self.round(&positional()),
            Intrinsic::Clamp => self.clamp(&arg(0), &arg(1), &arg(2)),
            Intrinsic::Rand => self.rand(&positional()),
            Intrinsic::Prob => self.prob(&arg(0)),
            Intrinsic::GetStep => self.get_step(&positional()),
            Intrinsic::GetDir | Intrinsic::GetDist => self.get_dir_dist(intrinsic, arg(0), arg(1)),
            Intrinsic::Locate => self.locate(&positional()),
            Intrinsic::Turn => self.turn(&arg(0), &arg(1)),
            Intrinsic::Rgb
                if (3..=4).contains(&args.len()) && args.iter().all(|(_, a)| matches!(a, GenericValue::Num(_))) =>
            {
                self.rgb(&positional())
            },
            Intrinsic::Rgb => Err(self.fault(FaultKind::Blocked(name.into()))),
            Intrinsic::Rgb2num
                if args
                    .iter()
                    .skip(1)
                    .all(|(_, space)| matches!(space, GenericValue::Null) || space.num() == Some(0.0)) =>
            {
                self.rgb2num(name, arg(0))
            },
            Intrinsic::Rgb2num => Err(self.fault(FaultKind::Blocked(name.into()))),
            Intrinsic::Num2text => self.num2text(&arg(0)),
            Intrinsic::Text2num => Ok(self.text2num(text(0))),
            Intrinsic::Uppertext => self.uppertext(text(0)),
            Intrinsic::Lowertext => self.lowertext(text(0)),
            Intrinsic::Sorttext | Intrinsic::SorttextEx => self.sorttext(intrinsic, text(0), text(1)),
            Intrinsic::Copytext | Intrinsic::CopytextChar => self.copytext(&positional()),
            Intrinsic::Findtext | Intrinsic::FindtextEx | Intrinsic::Findlasttext | Intrinsic::FindlasttextEx => {
                self.findtext(intrinsic, &positional())
            },
            Intrinsic::Replacetext | Intrinsic::ReplacetextEx => self.replacetext(intrinsic, &positional()),
            Intrinsic::Splittext => self.splittext(text(0), text(1)),
            Intrinsic::Params2list => self.params2list(text(0)),
            Intrinsic::JsonDecode => self.json_decode(text(0)),
            Intrinsic::Ref => self.ref_text(&arg(0)),
            Intrinsic::Hascall => Ok(self.hascall(arg(0), text(1)).into()),
            Intrinsic::Crash => self.crash(arg(0)),
            Intrinsic::Icon => self.icon(params, args),
            Intrinsic::Sound => self.sound(params, args),
            Intrinsic::Matrix => self.dm_matrix(positional()),
            Intrinsic::Regex => self.regex(&positional()),

            Intrinsic::Trimtext => self.trimtext(text(0)),
            Intrinsic::Ckey | Intrinsic::CkeyEx => self.ckey(intrinsic, text(0)),
            Intrinsic::Cmptext | Intrinsic::CmptextEx => self.cmptext(intrinsic, &positional()),
            Intrinsic::Addtext => self.addtext(&positional()),
            Intrinsic::Jointext => self.jointext(&positional()),
            Intrinsic::Text2ascii | Intrinsic::Text2asciiChar => self.text2ascii(&positional()),
            Intrinsic::Ascii2text => self.ascii2text(&arg(0)),

            Intrinsic::Sin => self.sin(&arg(0)),
            Intrinsic::Cos => self.cos(&arg(0)),
            Intrinsic::Tan => self.tan(&arg(0)),
            Intrinsic::Arcsin => self.arcsin(&arg(0)),
            Intrinsic::Arccos => self.arccos(&arg(0)),
            Intrinsic::Arctan => self.arctan(&positional()),
            Intrinsic::Log => self.log(&positional()),
            Intrinsic::Sign => self.sign(&arg(0)),
            Intrinsic::Lerp => self.lerp(&arg(0), &arg(1), &arg(2)),
            Intrinsic::Fract => self.fract(&arg(0)),
            Intrinsic::Trunc => self.trunc(&arg(0)),
            Intrinsic::Isnan => self.isnan(&arg(0)),
            Intrinsic::Isinf => self.isinf(&arg(0)),

            Intrinsic::Alist => self.dm_alist(&args),

            Intrinsic::Alert
            | Intrinsic::Astype
            | Intrinsic::BoundPixloc
            | Intrinsic::BoundsDist
            | Intrinsic::Bounds
            | Intrinsic::DmDbClose
            | Intrinsic::DmDbColumns
            | Intrinsic::DmDbConnect
            | Intrinsic::DmDbErrorMsg
            | Intrinsic::DmDbExecute
            | Intrinsic::DmDbIsConnected
            | Intrinsic::DmDbNewCon
            | Intrinsic::DmDbNewQuery
            | Intrinsic::DmDbNextRow
            | Intrinsic::DmDbQuote
            | Intrinsic::DmDbRowCount
            | Intrinsic::DmDbRowsAffected
            | Intrinsic::Filter
            | Intrinsic::FindlasttextChar
            | Intrinsic::FindlasttextExChar
            | Intrinsic::FindtextChar
            | Intrinsic::FindtextExChar
            | Intrinsic::Ftime
            | Intrinsic::GetStepAway
            | Intrinsic::GetStepRand
            | Intrinsic::GetStepsTo
            | Intrinsic::GetStepTo
            | Intrinsic::GetStepTowards
            | Intrinsic::Generator
            | Intrinsic::Gradient
            | Intrinsic::Hearers
            | Intrinsic::HtmlDecode
            | Intrinsic::HtmlEncode
            | Intrinsic::Ispointer
            | Intrinsic::JsonEncode
            | Intrinsic::Lentext
            | Intrinsic::List2params
            | Intrinsic::LoadExt
            | Intrinsic::LoadResource
            | Intrinsic::Md5
            | Intrinsic::Missile
            | Intrinsic::Newlist
            | Intrinsic::NoiseHash
            | Intrinsic::NonspantextChar
            | Intrinsic::Nonspantext
            | Intrinsic::Obounds
            | Intrinsic::Ohearers
            | Intrinsic::Output
            | Intrinsic::Oviewers
            | Intrinsic::Pixloc
            | Intrinsic::RandSeed
            | Intrinsic::Refcount
            | Intrinsic::ReplacetextChar
            | Intrinsic::ReplacetextExChar
            | Intrinsic::Roll
            | Intrinsic::Run
            | Intrinsic::Sha1
            | Intrinsic::SpantextChar
            | Intrinsic::Spantext
            | Intrinsic::SplicetextChar
            | Intrinsic::Splicetext
            | Intrinsic::SplittextChar
            | Intrinsic::Startup
            | Intrinsic::Stat
            | Intrinsic::Statpanel
            | Intrinsic::StepAway
            | Intrinsic::StepRand
            | Intrinsic::Step
            | Intrinsic::StepTo
            | Intrinsic::StepTowards
            | Intrinsic::Time2text
            | Intrinsic::UrlDecode
            | Intrinsic::UrlEncode
            | Intrinsic::ValuesCutOver
            | Intrinsic::ValuesCutUnder
            | Intrinsic::ValuesDot
            | Intrinsic::ValuesProduct
            | Intrinsic::ValuesSum
            | Intrinsic::Vector
            | Intrinsic::Viewers
            | Intrinsic::WalkAway
            | Intrinsic::WalkRand
            | Intrinsic::Walk
            | Intrinsic::WalkTo
            | Intrinsic::WalkTowards
            | Intrinsic::Winclone
            | Intrinsic::Winexists
            | Intrinsic::Winshow => Err(self.fault(FaultKind::Blocked(name.into()))),
        }
    }
}
