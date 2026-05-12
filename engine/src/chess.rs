use anyhow::{Error as E, Result};
use std::fmt;
use std::sync::OnceLock;

type Bitboard = u64;
const MAX_MOVES: usize = 256;

fn rank_mask(rank: u8) -> Bitboard {
    0xFF_u64 << (rank as usize * 8)
}

#[derive(Debug, Clone)]
pub struct FenError(String);

impl FenError {
    pub fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

impl fmt::Display for FenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for FenError {}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum Color {
    White = 0,
    Black,
}

impl Color {
    fn fen_active(self) -> char {
        match self {
            Color::White => 'w',
            Color::Black => 'b',
        }
    }

    fn opponent(self) -> Color {
        match self {
            Color::White => Color::Black,
            Color::Black => Color::White,
        }
    }

    fn idx(self) -> usize {
        self as usize
    }

    fn from_idx(idx: usize) -> Self {
        match idx {
            0 => Color::White,
            1 => Color::Black,
            _ => panic!("invalid color idx {idx}"),
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum PieceKind {
    Pawn = 0,
    Knight,
    Bishop,
    Rook,
    Queen,
    King,
}

impl From<usize> for PieceKind {
    fn from(v: usize) -> Self {
        match v {
            0 => Self::Pawn,
            1 => Self::Knight,
            2 => Self::Bishop,
            3 => Self::Rook,
            4 => Self::Queen,
            5 => Self::King,
            _ => panic!("Cannot convert from {} to PieceKind.", v),
        }
    }
}

impl PieceKind {
    fn idx(self) -> usize {
        self as usize
    }

    fn all() -> [PieceKind; 6] {
        [
            PieceKind::Pawn,
            PieceKind::Knight,
            PieceKind::Bishop,
            PieceKind::Rook,
            PieceKind::Queen,
            PieceKind::King,
        ]
    }
}

const PIECE_VALUES: [i32; 6] = [100, 320, 330, 500, 900, 30_000];

pub fn piece_value(kind: PieceKind) -> i32 {
    PIECE_VALUES[kind.idx()]
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct Piece {
    pub color: Color,
    pub kind: PieceKind,
}

impl Piece {
    pub const fn new(color: Color, kind: PieceKind) -> Self {
        Self { color, kind }
    }

    fn to_fen_symbol(self) -> char {
        let symbol = match self.kind {
            PieceKind::Pawn => 'p',
            PieceKind::Knight => 'n',
            PieceKind::Bishop => 'b',
            PieceKind::Rook => 'r',
            PieceKind::Queen => 'q',
            PieceKind::King => 'k',
        };

        match self.color {
            Color::White => symbol.to_ascii_uppercase(),
            Color::Black => symbol,
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct Square {
    rank: u8,
    file: u8,
}

impl Square {
    pub fn new(rank: u8, file: u8) -> Option<Self> {
        if rank < 8 && file < 8 {
            Some(Self { rank, file })
        } else {
            None
        }
    }

    pub const fn unchecked(rank: u8, file: u8) -> Self {
        Self { rank, file }
    }

    pub fn rank(self) -> u8 {
        self.rank
    }

    pub fn file(self) -> u8 {
        self.file
    }

    fn to_index(self) -> usize {
        self.rank as usize * 8 + self.file as usize
    }

    pub fn from_index(idx: u8) -> Self {
        let rank = idx / 8;
        let file = idx % 8;
        Square::unchecked(rank, file)
    }

    pub fn to_algebraic(self) -> String {
        let rank_char = (b'1' + self.rank) as char;
        let file_char = (b'a' + self.file) as char;
        format!("{file_char}{rank_char}")
    }

    pub fn from_algebraic(value: &str) -> Option<Self> {
        let bytes = value.as_bytes();
        if bytes.len() != 2 {
            return None;
        }

        let file = bytes[0];
        let rank = bytes[1];

        if !(b'a'..=b'h').contains(&file) || !(b'1'..=b'8').contains(&rank) {
            return None;
        }

        let rank_idx = rank - b'1';
        let file_idx = file - b'a';
        Some(Self {
            rank: rank_idx,
            file: file_idx,
        })
    }

    pub fn offset(self, dr: i8, df: i8) -> Option<Self> {
        let rank = self.rank as i8 + dr;
        let file = self.file as i8 + df;
        if (0..=7).contains(&rank) && (0..=7).contains(&file) {
            Some(Square::unchecked(rank as u8, file as u8))
        } else {
            None
        }
    }
}

impl fmt::Display for Square {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_algebraic())
    }
}

fn square_bitboard(square: Square) -> Bitboard {
    1u64 << square.to_index()
}

fn for_each_bit(mut bb: Bitboard, mut f: impl FnMut(Square)) {
    while bb != 0 {
        let idx = bb.trailing_zeros() as u8;
        bb &= bb - 1;
        f(Square::from_index(idx));
    }
}

fn orient_square(square: Square, perspective: Color) -> Square {
    if perspective == Color::White {
        square
    } else {
        Square::unchecked(7 - square.rank(), square.file())
    }
}

fn orient_color(color: Color, perspective: Color) -> Color {
    if perspective == Color::White {
        color
    } else {
        color.opponent()
    }
}

const KNIGHT_DELTAS: [(i8, i8); 8] = [
    (1, 2),
    (2, 1),
    (2, -1),
    (1, -2),
    (-1, -2),
    (-2, -1),
    (-2, 1),
    (-1, 2),
];

const KING_DELTAS: [(i8, i8); 8] = [
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
    (0, -1),
    (1, -1),
];

const BISHOP_DIRS: [(i8, i8); 4] = [(1, 1), (1, -1), (-1, 1), (-1, -1)];
const ROOK_DIRS: [(i8, i8); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

fn knight_masks() -> &'static [Bitboard; 64] {
    static KNIGHT: OnceLock<[Bitboard; 64]> = OnceLock::new();
    KNIGHT.get_or_init(|| {
        let mut table = [0_u64; 64];
        for (idx, entry) in table.iter_mut().enumerate() {
            let sq = Square::from_index(idx as u8);
            let mut mask = 0;
            for (dr, df) in KNIGHT_DELTAS {
                if let Some(target) = sq.offset(dr, df) {
                    mask |= square_bitboard(target);
                }
            }
            *entry = mask;
        }
        table
    })
}

fn king_masks() -> &'static [Bitboard; 64] {
    static KING: OnceLock<[Bitboard; 64]> = OnceLock::new();
    KING.get_or_init(|| {
        let mut table = [0_u64; 64];
        for (idx, entry) in table.iter_mut().enumerate() {
            let sq = Square::from_index(idx as u8);
            let mut mask = 0;
            for (dr, df) in KING_DELTAS {
                if let Some(target) = sq.offset(dr, df) {
                    mask |= square_bitboard(target);
                }
            }
            *entry = mask;
        }
        table
    })
}

fn pawn_attack_masks() -> &'static [[Bitboard; 64]; 2] {
    static MASKS: OnceLock<[[Bitboard; 64]; 2]> = OnceLock::new();
    MASKS.get_or_init(|| {
        let mut table = [[0_u64; 64]; 2];
        for (idx, entry) in table[Color::White.idx()].iter_mut().enumerate() {
            let sq = Square::from_index(idx as u8);
            let mut white_mask = 0;
            for df in [-1, 1] {
                if let Some(target) = sq.offset(1, df) {
                    white_mask |= square_bitboard(target);
                }
            }
            *entry = white_mask;
        }
        for (idx, entry) in table[Color::Black.idx()].iter_mut().enumerate() {
            let sq = Square::from_index(idx as u8);
            let mut black_mask = 0;
            for df in [-1, 1] {
                if let Some(target) = sq.offset(-1, df) {
                    black_mask |= square_bitboard(target);
                }
            }
            *entry = black_mask;
        }
        table
    })
}

fn knight_attacks(square: Square) -> Bitboard {
    knight_masks()[square.to_index()]
}

fn king_attacks(square: Square) -> Bitboard {
    king_masks()[square.to_index()]
}

fn pawn_attacks(square: Square, color: Color) -> Bitboard {
    pawn_attack_masks()[color.idx()][square.to_index()]
}

fn sliding_attacks_from(
    square: Square,
    occupancy: Bitboard,
    dirs: &[(i8, i8)],
) -> Bitboard {
    let mut attacks = 0;
    for (dr, df) in dirs {
        let mut rank = square.rank() as i8 + dr;
        let mut file = square.file() as i8 + df;
        while (0..=7).contains(&rank) && (0..=7).contains(&file) {
            let target = Square::unchecked(rank as u8, file as u8);
            let mask = square_bitboard(target);
            attacks |= mask;
            if occupancy & mask != 0 {
                break;
            }
            rank += dr;
            file += df;
        }
    }
    attacks
}

fn direction_step(from: Square, to: Square) -> Option<(i8, i8)> {
    let dr = to.rank() as i8 - from.rank() as i8;
    let df = to.file() as i8 - from.file() as i8;
    if dr == 0 && df == 0 {
        return None;
    }
    if dr == 0 {
        return Some((0, df.signum()));
    }
    if df == 0 {
        return Some((dr.signum(), 0));
    }
    if dr.abs() == df.abs() {
        return Some((dr.signum(), df.signum()));
    }
    None
}

fn squares_between(from: Square, to: Square) -> Bitboard {
    let mut mask = 0;
    if let Some((dr, df)) = direction_step(from, to) {
        let mut current = from;
        while let Some(next) = current.offset(dr, df) {
            if next == to {
                break;
            }
            mask |= square_bitboard(next);
            current = next;
        }
    }
    mask
}

fn ray_mask_from(square: Square, dr: i8, df: i8) -> Bitboard {
    let mut mask = 0;
    let mut current = square;
    while let Some(next) = current.offset(dr, df) {
        mask |= square_bitboard(next);
        current = next;
    }
    mask
}

fn zobrist_keys() -> &'static ZobristKeys {
    static KEYS: OnceLock<ZobristKeys> = OnceLock::new();
    KEYS.get_or_init(ZobristKeys::new)
}

struct ZobristKeys {
    pieces: [[[u64; 64]; 6]; 2],
    side_to_move: u64,
    castling: [u64; 4],
    en_passant: [u64; 8],
}

impl ZobristKeys {
    fn new() -> Self {
        let mut rng = SplitMix64::new(0xDEAD_BEEF_F00D_BAAD);
        let mut pieces = [[[0_u64; 64]; 6]; 2];
        for color in &mut pieces {
            for kind in color.iter_mut() {
                for square in kind.iter_mut() {
                    *square = rng.next_u64();
                }
            }
        }

        let mut castling = [0_u64; 4];
        for entry in &mut castling {
            *entry = rng.next_u64();
        }

        let mut en_passant = [0_u64; 8];
        for entry in &mut en_passant {
            *entry = rng.next_u64();
        }

        let side_to_move = rng.next_u64();

        Self {
            pieces,
            side_to_move,
            castling,
            en_passant,
        }
    }
}

struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15_u64);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct Move {
    pub from: Square,
    pub to: Square,
    promotion: bool,
}

impl Move {
    pub const fn new(from: Square, to: Square) -> Self {
        Self {
            from,
            to,
            promotion: false,
        }
    }

    pub const fn with_promotion(from: Square, to: Square) -> Self {
        Self {
            from,
            to,
            promotion: true,
        }
    }

    pub fn to_uci(self) -> String {
        let mut text =
            format!("{}{}", self.from.to_algebraic(), self.to.to_algebraic());
        if self.promotion {
            text.push('q');
        }
        text
    }

    pub const fn empty() -> Self {
        Self {
            from: Square::unchecked(0, 0),
            to: Square::unchecked(0, 0),
            promotion: false,
        }
    }

    pub fn is_promotion(self) -> bool {
        self.promotion
    }
}

#[derive(Copy, Clone, Debug)]
pub struct MoveUndo {
    pub(crate) mv: Move,
    pub(crate) moving_piece: Piece,
    pub(crate) captured_piece: Option<Piece>,
    pub(crate) prev_castling_rights: CastlingRights,
    pub(crate) prev_en_passant: Option<Square>,
    pub(crate) prev_halfmove_clock: u32,
    pub(crate) prev_fullmove_number: u32,
    pub(crate) prev_active_color: Color,
}

#[derive(Copy, Clone, Debug)]
pub struct NullMoveUndo {
    prev_en_passant: Option<Square>,
    prev_halfmove_clock: u32,
    prev_fullmove_number: u32,
    prev_active_color: Color,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Default)]
pub struct CastlingRights {
    pub white_kingside: bool,
    pub white_queenside: bool,
    pub black_kingside: bool,
    pub black_queenside: bool,
}

impl CastlingRights {
    pub fn as_fen(self) -> String {
        let mut buffer = String::with_capacity(4);
        if self.white_kingside {
            buffer.push('K');
        }
        if self.white_queenside {
            buffer.push('Q');
        }
        if self.black_kingside {
            buffer.push('k');
        }
        if self.black_queenside {
            buffer.push('q');
        }

        if buffer.is_empty() {
            "-".to_string()
        } else {
            buffer
        }
    }

    fn validates_for(self, board: &Board) -> bool {
        // Rely on classical chess: castling requires king and rooks to exist on home squares.
        let white_king_ok =
            board.has_piece(Square::unchecked(0, 4), PieceKind::King, Color::White);
        let black_king_ok =
            board.has_piece(Square::unchecked(7, 4), PieceKind::King, Color::Black);

        if self.white_kingside
            && !(white_king_ok
                && board.has_piece(
                    Square::unchecked(0, 7),
                    PieceKind::Rook,
                    Color::White,
                ))
        {
            return false;
        }
        if self.white_queenside
            && !(white_king_ok
                && board.has_piece(
                    Square::unchecked(0, 0),
                    PieceKind::Rook,
                    Color::White,
                ))
        {
            return false;
        }
        if self.black_kingside
            && !(black_king_ok
                && board.has_piece(
                    Square::unchecked(7, 7),
                    PieceKind::Rook,
                    Color::Black,
                ))
        {
            return false;
        }
        if self.black_queenside
            && !(black_king_ok
                && board.has_piece(
                    Square::unchecked(7, 0),
                    PieceKind::Rook,
                    Color::Black,
                ))
        {
            return false;
        }

        true
    }
}

fn attackers_to_with(
    square: Square,
    occupancy: Bitboard,
    bitboards: &[[Bitboard; 6]; 2],
) -> Bitboard {
    let mut attackers = 0;
    attackers |= pawn_attacks(square, Color::Black)
        & bitboards[Color::White.idx()][PieceKind::Pawn.idx()];
    attackers |= pawn_attacks(square, Color::White)
        & bitboards[Color::Black.idx()][PieceKind::Pawn.idx()];
    attackers |= knight_attacks(square)
        & (bitboards[Color::White.idx()][PieceKind::Knight.idx()]
            | bitboards[Color::Black.idx()][PieceKind::Knight.idx()]);
    let bishop_attacks = sliding_attacks_from(square, occupancy, &BISHOP_DIRS);
    attackers |= bishop_attacks
        & (bitboards[Color::White.idx()][PieceKind::Bishop.idx()]
            | bitboards[Color::White.idx()][PieceKind::Queen.idx()]
            | bitboards[Color::Black.idx()][PieceKind::Bishop.idx()]
            | bitboards[Color::Black.idx()][PieceKind::Queen.idx()]);
    let rook_attacks = sliding_attacks_from(square, occupancy, &ROOK_DIRS);
    attackers |= rook_attacks
        & (bitboards[Color::White.idx()][PieceKind::Rook.idx()]
            | bitboards[Color::White.idx()][PieceKind::Queen.idx()]
            | bitboards[Color::Black.idx()][PieceKind::Rook.idx()]
            | bitboards[Color::Black.idx()][PieceKind::Queen.idx()]);
    attackers |= king_attacks(square)
        & (bitboards[Color::White.idx()][PieceKind::King.idx()]
            | bitboards[Color::Black.idx()][PieceKind::King.idx()]);
    attackers
}

fn smallest_attacker_from(
    attackers: Bitboard,
    color: Color,
    bitboards: &[[Bitboard; 6]; 2],
) -> Option<(PieceKind, Square)> {
    for kind in PieceKind::all() {
        let bb = attackers & bitboards[color.idx()][kind.idx()];
        if bb != 0 {
            let sq = Square::from_index(bb.trailing_zeros() as u8);
            return Some((kind, sq));
        }
    }
    None
}

fn square_attacked_state(
    square: Square,
    by: Color,
    bitboards: &[[Bitboard; 6]; 2],
    occupancy: Bitboard,
) -> bool {
    let idx = by.idx();
    let pawns = bitboards[idx][PieceKind::Pawn.idx()];
    if pawns & pawn_attacks(square, by.opponent()) != 0 {
        return true;
    }

    let knights = bitboards[idx][PieceKind::Knight.idx()];
    if knights & knight_attacks(square) != 0 {
        return true;
    }

    let king = bitboards[idx][PieceKind::King.idx()];
    if king & king_attacks(square) != 0 {
        return true;
    }

    let bishops = bitboards[idx][PieceKind::Bishop.idx()];
    let rooks = bitboards[idx][PieceKind::Rook.idx()];
    let queens = bitboards[idx][PieceKind::Queen.idx()];

    if sliding_attacks_from(square, occupancy, &BISHOP_DIRS) & (bishops | queens) != 0
    {
        return true;
    }

    if sliding_attacks_from(square, occupancy, &ROOK_DIRS) & (rooks | queens) != 0 {
        return true;
    }

    false
}

#[derive(Clone)]
pub struct Board {
    piece_bitboards: [[Bitboard; 6]; 2],
    occupancy: Bitboard,
    occupancy_by_color: [Bitboard; 2],
    nnue: [NnueAccumulator; 2],
    movegen_cache: [Option<MoveGenCacheEntry>; 2],
    pub active_color: Color,
    pub castling_rights: CastlingRights,
    pub en_passant: Option<Square>,
    pub halfmove_clock: u32,
    pub fullmove_number: u32,
    zobrist: u64,
}

#[derive(Clone)]
pub struct MoveList {
    data: [Move; MAX_MOVES],
    len: usize,
}

#[derive(Clone, Copy)]
struct MoveGenCacheEntry {
    king_square: Square,
    response_mask: Bitboard,
    check_count: u32,
    pin_masks: [Bitboard; 64],
}

impl MoveGenCacheEntry {
    fn new(
        king_square: Square,
        response_mask: Bitboard,
        check_count: u32,
        pin_masks: [Bitboard; 64],
    ) -> Self {
        Self {
            king_square,
            response_mask,
            check_count,
            pin_masks,
        }
    }
}

impl MoveList {
    pub fn new() -> Self {
        Self {
            data: [Move::empty(); MAX_MOVES],
            len: 0,
        }
    }

    pub fn push(&mut self, mv: Move) {
        if self.len < MAX_MOVES {
            self.data[self.len] = mv;
            self.len += 1;
        }
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Move> {
        self.data[..self.len].iter()
    }

    pub fn as_slice(&self) -> &[Move] {
        &self.data[..self.len]
    }

    pub fn as_mut_slice(&mut self) -> &mut [Move] {
        &mut self.data[..self.len]
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn contains(&self, mv: &Move) -> bool {
        self.as_slice().contains(mv)
    }
}

impl Default for MoveList {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
struct NnueAccumulator {
    perspective: Color,
    king_square: Option<Square>,
    active: Vec<i64>,
}

impl NnueAccumulator {
    fn new(perspective: Color) -> Self {
        Self {
            perspective,
            king_square: None,
            active: Vec::new(),
        }
    }

    fn rebuild(&mut self, board: &Board) {
        self.active.clear();
        let king_color = if self.perspective == Color::White {
            Color::White
        } else {
            Color::Black
        };
        self.king_square = board
            .king_square(king_color)
            .map(|sq| orient_square(sq, self.perspective));
        if self.king_square.is_none() {
            return;
        }
        for color in [Color::White, Color::Black] {
            for kind in PieceKind::all() {
                let mut bb = board.piece_bitboards[color.idx()][kind.idx()];
                while bb != 0 {
                    let idx = bb.trailing_zeros() as u8;
                    bb &= bb - 1;
                    let sq = Square::from_index(idx);
                    if let Some(feature) =
                        self.feature_index(color, kind, sq, self.king_square.unwrap())
                    {
                        self.active.push(feature);
                    }
                }
            }
        }
    }

    fn add_piece(&mut self, color: Color, kind: PieceKind, square: Square) {
        if let Some(king_sq) = self.king_square {
            if let Some(feature) = self.feature_index(color, kind, square, king_sq) {
                self.active.push(feature);
            }
        }
    }

    fn remove_piece(&mut self, color: Color, kind: PieceKind, square: Square) {
        if let Some(king_sq) = self.king_square {
            if let Some(feature) = self.feature_index(color, kind, square, king_sq) {
                if let Some(pos) = self.active.iter().position(|&v| v == feature) {
                    self.active.swap_remove(pos);
                }
            }
        }
    }

    fn feature_index(
        &self,
        piece_color: Color,
        kind: PieceKind,
        square: Square,
        king_sq: Square,
    ) -> Option<i64> {
        let oriented_sq = orient_square(square, self.perspective);
        let oriented_color = orient_color(piece_color, self.perspective);
        let plane = kind.idx() + if oriented_color == Color::White { 0 } else { 6 };
        let index =
            king_sq.to_index() * (12 * 64) + plane * 64 + oriented_sq.to_index();
        Some(index as i64)
    }
}

impl Default for Board {
    fn default() -> Self {
        Self::starting_position()
    }
}

impl Board {
    pub fn empty() -> Self {
        Self {
            piece_bitboards: [[0; 6]; 2],
            occupancy: 0,
            occupancy_by_color: [0; 2],
            nnue: [
                NnueAccumulator::new(Color::White),
                NnueAccumulator::new(Color::Black),
            ],
            movegen_cache: [None, None],
            active_color: Color::White,
            castling_rights: CastlingRights::default(),
            en_passant: None,
            halfmove_clock: 0,
            fullmove_number: 1,
            zobrist: 0,
        }
    }

    fn invalidate_movegen_cache(&mut self) {
        self.movegen_cache = [None, None];
    }

    fn movegen_entry(&mut self, color: Color) -> Option<&MoveGenCacheEntry> {
        if self.movegen_cache[color.idx()].is_none() {
            let entry = self.build_movegen_entry(color)?;
            self.movegen_cache[color.idx()] = Some(entry);
        }
        self.movegen_cache[color.idx()].as_ref()
    }

    fn build_movegen_entry(&self, color: Color) -> Option<MoveGenCacheEntry> {
        let king_square = self.king_square(color)?;
        let enemy = color.opponent();
        let occupancy = self.occupancy;
        let enemy_pawns = self.piece_bitboards[enemy.idx()][PieceKind::Pawn.idx()];
        let enemy_knights =
            self.piece_bitboards[enemy.idx()][PieceKind::Knight.idx()];
        let enemy_bishops =
            self.piece_bitboards[enemy.idx()][PieceKind::Bishop.idx()];
        let enemy_rooks = self.piece_bitboards[enemy.idx()][PieceKind::Rook.idx()];
        let enemy_queens = self.piece_bitboards[enemy.idx()][PieceKind::Queen.idx()];

        let pawn_checkers = enemy_pawns & pawn_attacks(king_square, color);
        let knight_checkers = enemy_knights & knight_attacks(king_square);
        let bishop_sliders = enemy_bishops | enemy_queens;
        let rook_sliders = enemy_rooks | enemy_queens;
        let bishop_attacks =
            sliding_attacks_from(king_square, occupancy, &BISHOP_DIRS);
        let rook_attacks = sliding_attacks_from(king_square, occupancy, &ROOK_DIRS);
        let bishop_checkers = bishop_attacks & bishop_sliders;
        let rook_checkers = rook_attacks & rook_sliders;
        let checkers =
            pawn_checkers | knight_checkers | bishop_checkers | rook_checkers;
        let check_count = checkers.count_ones();

        let mut response_mask: Bitboard = !0;
        if check_count == 1 {
            let checker_sq = Square::from_index(checkers.trailing_zeros() as u8);
            response_mask = square_bitboard(checker_sq);
            if (bishop_checkers | rook_checkers) & response_mask != 0 {
                response_mask |= squares_between(king_square, checker_sq);
            }
        } else if check_count >= 2 {
            response_mask = 0;
        }

        let mut pin_masks = [!0u64; 64];
        self.compute_pin_masks(
            color,
            king_square,
            bishop_sliders,
            rook_sliders,
            &mut pin_masks,
        );

        Some(MoveGenCacheEntry::new(
            king_square,
            response_mask,
            check_count,
            pin_masks,
        ))
    }

    pub fn starting_position() -> Self {
        use Color::{Black, White};
        use PieceKind::*;

        let mut board = Board::empty();

        let back_rank = [Rook, Knight, Bishop, Queen, King, Bishop, Knight, Rook];

        for (file, kind) in back_rank.into_iter().enumerate() {
            board.set_piece(
                Square::unchecked(0, file as u8),
                Some(Piece::new(White, kind)),
            );
            board.set_piece(
                Square::unchecked(1, file as u8),
                Some(Piece::new(White, Pawn)),
            );
            board.set_piece(
                Square::unchecked(6, file as u8),
                Some(Piece::new(Black, Pawn)),
            );
            board.set_piece(
                Square::unchecked(7, file as u8),
                Some(Piece::new(Black, kind)),
            );
        }

        board.set_castling_rights(CastlingRights {
            white_kingside: true,
            white_queenside: true,
            black_kingside: true,
            black_queenside: true,
        });
        board.rebuild_nnue_state();
        board
    }

    pub fn set_piece(&mut self, square: Square, piece: Option<Piece>) {
        self.invalidate_movegen_cache();
        let idx = square.to_index();
        let keys = zobrist_keys();
        if let Some(old) = self.remove_piece_internal(square) {
            self.zobrist ^= keys.pieces[old.color.idx()][old.kind.idx()][idx];
            self.nnue_remove_piece(old, square);
        }
        if let Some(p) = piece {
            self.place_piece_internal(square, p);
            self.zobrist ^= keys.pieces[p.color.idx()][p.kind.idx()][idx];
            self.nnue_add_piece(p, square);
            if p.kind == PieceKind::King {
                self.rebuild_nnue_for_color(p.color);
            }
        }
    }

    pub fn piece_at(&self, square: Square) -> Option<Piece> {
        let mask = square_bitboard(square);
        for color_idx in 0..2 {
            if self.occupancy_by_color[color_idx] & mask != 0 {
                for kind_idx in 0..6 {
                    if self.piece_bitboards[color_idx][kind_idx] & mask != 0 {
                        return Some(Piece::new(
                            Color::from_idx(color_idx),
                            PieceKind::from(kind_idx),
                        ));
                    }
                }
            }
        }
        None
    }

    pub fn piece_count(&self) -> u32 {
        self.occupancy.count_ones()
    }

    fn remove_piece_internal(&mut self, square: Square) -> Option<Piece> {
        let mask = square_bitboard(square);
        for color_idx in 0..2 {
            if self.occupancy_by_color[color_idx] & mask != 0 {
                for kind_idx in 0..6 {
                    if self.piece_bitboards[color_idx][kind_idx] & mask != 0 {
                        self.piece_bitboards[color_idx][kind_idx] &= !mask;
                        self.occupancy_by_color[color_idx] &= !mask;
                        self.occupancy &= !mask;
                        return Some(Piece::new(
                            Color::from_idx(color_idx),
                            PieceKind::from(kind_idx),
                        ));
                    }
                }
            }
        }
        None
    }

    fn place_piece_internal(&mut self, square: Square, piece: Piece) {
        let mask = square_bitboard(square);
        let color_idx = piece.color.idx();
        let kind_idx = piece.kind.idx();
        self.piece_bitboards[color_idx][kind_idx] |= mask;
        self.occupancy_by_color[color_idx] |= mask;
        self.occupancy |= mask;
    }

    fn nnue_remove_piece(&mut self, piece: Piece, square: Square) {
        for perspective in [Color::White, Color::Black] {
            self.nnue[perspective.idx()].remove_piece(
                piece.color,
                piece.kind,
                square,
            );
            if piece.kind == PieceKind::King && perspective == piece.color {
                self.nnue[perspective.idx()].king_square = None;
            }
        }
    }

    fn nnue_add_piece(&mut self, piece: Piece, square: Square) {
        for perspective in [Color::White, Color::Black] {
            if piece.kind == PieceKind::King && perspective == piece.color {
                self.nnue[perspective.idx()].king_square =
                    Some(orient_square(square, perspective));
                self.rebuild_nnue_for_color(piece.color);
                return;
            }
            self.nnue[perspective.idx()].add_piece(piece.color, piece.kind, square);
        }
    }

    pub fn set_active_color(&mut self, color: Color) {
        if self.active_color != color {
            self.zobrist ^= zobrist_keys().side_to_move;
            self.active_color = color;
        }
    }

    pub fn set_en_passant(&mut self, square: Option<Square>) {
        if self.en_passant == square {
            return;
        }
        self.invalidate_movegen_cache();
        let keys = zobrist_keys();
        if let Some(old) = self.en_passant {
            self.zobrist ^= keys.en_passant[old.file() as usize];
        }
        self.en_passant = square;
        if let Some(new_sq) = self.en_passant {
            self.zobrist ^= keys.en_passant[new_sq.file() as usize];
        }
    }

    pub fn set_castling_rights(&mut self, rights: CastlingRights) {
        if self.castling_rights == rights {
            return;
        }
        self.invalidate_movegen_cache();
        let old = self.castling_rights;
        self.castling_rights = rights;
        self.update_castling_hash(old, rights);
    }

    fn toggle_active_color(&mut self) {
        self.zobrist ^= zobrist_keys().side_to_move;
        self.active_color = self.active_color.opponent();
    }

    fn update_castling_hash(&mut self, old: CastlingRights, new: CastlingRights) {
        let keys = zobrist_keys();
        if old.white_kingside != new.white_kingside {
            self.zobrist ^= keys.castling[0];
        }
        if old.white_queenside != new.white_queenside {
            self.zobrist ^= keys.castling[1];
        }
        if old.black_kingside != new.black_kingside {
            self.zobrist ^= keys.castling[2];
        }
        if old.black_queenside != new.black_queenside {
            self.zobrist ^= keys.castling[3];
        }
    }

    pub fn play_move(&mut self, mv: Move) -> bool {
        if self.move_is_legal(mv) {
            self.make_move(mv);
            true
        } else {
            false
        }
    }

    fn has_piece(&self, square: Square, kind: PieceKind, color: Color) -> bool {
        self.piece_at(square) == Some(Piece::new(color, kind))
    }

    pub fn hash(&self) -> u64 {
        self.zobrist
    }

    pub fn material_count(&self) -> i32 {
        let mut total = 0;
        for color_idx in 0..2 {
            for kind in PieceKind::all() {
                let bb = self.piece_bitboards[color_idx][kind.idx()];
                total += piece_value(kind) * bb.count_ones() as i32;
            }
        }
        total
    }

    fn rebuild_nnue_state(&mut self) {
        let mut white = NnueAccumulator::new(Color::White);
        white.rebuild(self);
        let mut black = NnueAccumulator::new(Color::Black);
        black.rebuild(self);
        self.nnue[Color::White.idx()] = white;
        self.nnue[Color::Black.idx()] = black;
    }

    fn rebuild_nnue_for_color(&mut self, color: Color) {
        let mut acc = NnueAccumulator::new(color);
        acc.rebuild(self);
        self.nnue[color.idx()] = acc;
    }

    pub fn nnue_active_indices(&self) -> &[i64] {
        &self.nnue[self.active_color.idx()].active
    }

    pub fn from_fen(fen: &str) -> Result<Self, FenError> {
        let mut parts = fen.split_whitespace();
        let placement = parts
            .next()
            .ok_or_else(|| FenError::new("FEN missing piece placement field"))?;
        let active = parts
            .next()
            .ok_or_else(|| FenError::new("FEN missing active color field"))?;
        let castling = parts
            .next()
            .ok_or_else(|| FenError::new("FEN missing castling rights field"))?;
        let en_passant = parts
            .next()
            .ok_or_else(|| FenError::new("FEN missing en passant field"))?;
        let halfmove_field = parts.next();
        let fullmove_field = parts.next();
        if parts.next().is_some() {
            return Err(FenError::new("FEN has extra fields"));
        }

        let mut board = Board::empty();
        Self::parse_placement(placement, &mut board)?;
        let active_color = match active {
            "w" => Color::White,
            "b" => Color::Black,
            value => {
                return Err(FenError::new(format!("invalid active color '{value}'")));
            }
        };
        board.set_active_color(active_color);

        let castling_rights = Self::parse_castling(castling)?;
        board.set_castling_rights(castling_rights);
        let ep_square = if en_passant == "-" {
            None
        } else {
            Some(Square::from_algebraic(en_passant).ok_or_else(|| {
                FenError::new(format!("invalid en passant square '{en_passant}'"))
            })?)
        };
        board.set_en_passant(ep_square);

        board.halfmove_clock = match halfmove_field {
            Some(value) => value
                .parse::<u32>()
                .map_err(|_| FenError::new("invalid halfmove clock"))?,
            None => 0,
        };
        board.fullmove_number = match fullmove_field {
            Some(value) => value
                .parse::<u32>()
                .map_err(|_| FenError::new("invalid fullmove number"))?,
            None => 1,
        };
        if board.fullmove_number == 0 {
            return Err(FenError::new("fullmove number must be > 0"));
        }

        board.rebuild_nnue_state();
        Ok(board)
    }

    fn parse_castling(field: &str) -> Result<CastlingRights, FenError> {
        if field == "-" {
            return Ok(CastlingRights::default());
        }

        let mut rights = CastlingRights::default();
        for ch in field.chars() {
            match ch {
                'K' => rights.white_kingside = true,
                'Q' => rights.white_queenside = true,
                'k' => rights.black_kingside = true,
                'q' => rights.black_queenside = true,
                _ => {
                    return Err(FenError::new(format!(
                        "invalid castling flag '{ch}'"
                    )));
                }
            }
        }

        Ok(rights)
    }

    fn parse_placement(field: &str, board: &mut Board) -> Result<(), FenError> {
        let ranks: Vec<&str> = field.split('/').collect();
        if ranks.len() != 8 {
            return Err(FenError::new("piece placement must have 8 ranks"));
        }

        for (rank_idx, rank_data) in ranks.iter().rev().enumerate() {
            let mut file = 0;
            for ch in rank_data.chars() {
                if ch.is_ascii_digit() {
                    let empty = ch.to_digit(10).unwrap();
                    file += empty as usize;
                } else {
                    let piece = Self::piece_from_fen(ch).ok_or_else(|| {
                        FenError::new(format!("invalid piece char '{ch}'"))
                    })?;
                    if file >= 8 {
                        return Err(FenError::new("too many squares in rank"));
                    }
                    board.set_piece(
                        Square::unchecked(rank_idx as u8, file as u8),
                        Some(piece),
                    );
                    file += 1;
                }
            }

            if file != 8 {
                return Err(FenError::new("each rank must have 8 squares"));
            }
        }

        Ok(())
    }

    fn piece_from_fen(ch: char) -> Option<Piece> {
        let color = if ch.is_ascii_uppercase() {
            Color::White
        } else {
            Color::Black
        };
        let kind = match ch.to_ascii_lowercase() {
            'p' => PieceKind::Pawn,
            'n' => PieceKind::Knight,
            'b' => PieceKind::Bishop,
            'r' => PieceKind::Rook,
            'q' => PieceKind::Queen,
            'k' => PieceKind::King,
            _ => return None,
        };

        Some(Piece::new(color, kind))
    }

    pub fn fen(&self) -> String {
        let mut result = String::with_capacity(80);

        for rank in (0..8).rev() {
            let mut empty = 0;

            for file in 0..8 {
                let square = Square::unchecked(rank as u8, file as u8);
                match self.piece_at(square) {
                    Some(piece) => {
                        if empty > 0 {
                            result.push(char::from_digit(empty, 10).unwrap());
                            empty = 0;
                        }
                        result.push(piece.to_fen_symbol());
                    }
                    None => empty += 1,
                }
            }

            if empty > 0 {
                result.push(char::from_digit(empty, 10).unwrap());
            }

            if rank != 0 {
                result.push('/');
            }
        }

        result.push(' ');
        result.push(self.active_color.fen_active());
        result.push(' ');
        result.push_str(&self.castling_rights.as_fen());
        result.push(' ');

        match self.en_passant {
            Some(sq) => result.push_str(&sq.to_algebraic()),
            None => result.push('-'),
        }

        result.push(' ');
        result.push_str(&self.halfmove_clock.to_string());
        result.push(' ');
        result.push_str(&self.fullmove_number.to_string());

        result
    }

    pub fn is_legal(&self) -> bool {
        let white_kings = self.piece_bitboards[Color::White.idx()]
            [PieceKind::King.idx()]
        .count_ones();
        let black_kings = self.piece_bitboards[Color::Black.idx()]
            [PieceKind::King.idx()]
        .count_ones();
        if white_kings != 1 || black_kings != 1 {
            return false;
        }

        let home_rank_mask = rank_mask(0) | rank_mask(7);
        for color in [Color::White, Color::Black] {
            let pawns = self.piece_bitboards[color.idx()][PieceKind::Pawn.idx()];
            if pawns & home_rank_mask != 0 {
                return false;
            }
        }

        if !self.castling_rights.validates_for(self) {
            return false;
        }

        match self.en_passant {
            Some(square) => matches!(square.rank, 2 | 5),
            None => true,
        }
    }

    pub fn legal_moves_into(&mut self, buffer: &mut MoveList) {
        buffer.clear();
        let color = self.active_color;
        let Some(info) = self.movegen_entry(color).copied() else {
            return;
        };
        let king_square = info.king_square;
        let enemy = color.opponent();
        let friendly_occ = self.occupancy_by_color[color.idx()];
        let enemy_occ = self.occupancy_by_color[enemy.idx()];
        let occupancy = self.occupancy;
        let king_mask = square_bitboard(king_square);
        let response_mask = info.response_mask;
        let check_count = info.check_count;
        let pin_masks = info.pin_masks;

        let king_targets = king_attacks(king_square) & !friendly_occ;
        for_each_bit(king_targets, |target| {
            let to_mask = square_bitboard(target);
            let occ_prime = (occupancy & !king_mask) | to_mask;
            if !square_attacked_state(target, enemy, &self.piece_bitboards, occ_prime)
            {
                buffer.push(Move::new(king_square, target));
            }
        });

        if check_count >= 2 {
            return;
        }

        let mut pawns = self.piece_bitboards[color.idx()][PieceKind::Pawn.idx()];
        let pawn_dir: i8 = if color == Color::White { 1 } else { -1 };
        let pawn_start_rank = if color == Color::White { 1 } else { 6 };
        let promotion_rank = if color == Color::White { 7 } else { 0 };
        while pawns != 0 {
            let idx = pawns.trailing_zeros() as u8;
            pawns &= pawns - 1;
            let from = Square::from_index(idx);
            let allowed = response_mask & pin_masks[idx as usize];
            if allowed == 0 {
                continue;
            }

            if let Some(one_step) = from.offset(pawn_dir, 0) {
                let mask = square_bitboard(one_step);
                if occupancy & mask == 0 && allowed & mask != 0 {
                    if one_step.rank() == promotion_rank {
                        buffer.push(Move::with_promotion(from, one_step));
                    } else {
                        buffer.push(Move::new(from, one_step));
                    }
                    if from.rank() == pawn_start_rank {
                        if let Some(two_step) = one_step.offset(pawn_dir, 0) {
                            let two_mask = square_bitboard(two_step);
                            if occupancy & two_mask == 0 && allowed & two_mask != 0 {
                                buffer.push(Move::new(from, two_step));
                            }
                        }
                    }
                }
            }

            for df in [-1, 1] {
                if let Some(target) = from.offset(pawn_dir, df) {
                    let mask = square_bitboard(target);
                    if allowed & mask == 0 {
                        continue;
                    }
                    if enemy_occ & mask != 0 {
                        if target.rank() == promotion_rank {
                            buffer.push(Move::with_promotion(from, target));
                        } else {
                            buffer.push(Move::new(from, target));
                        }
                    }
                }
            }
        }

        let mut knights = self.piece_bitboards[color.idx()][PieceKind::Knight.idx()];
        while knights != 0 {
            let idx = knights.trailing_zeros() as u8;
            knights &= knights - 1;
            let from = Square::from_index(idx);
            let mut targets = knight_attacks(from) & !friendly_occ;
            targets &= response_mask & pin_masks[idx as usize];
            for_each_bit(targets, |target| buffer.push(Move::new(from, target)));
        }

        let mut bishops = self.piece_bitboards[color.idx()][PieceKind::Bishop.idx()];
        while bishops != 0 {
            let idx = bishops.trailing_zeros() as u8;
            bishops &= bishops - 1;
            let from = Square::from_index(idx);
            let mut targets =
                sliding_attacks_from(from, occupancy, &BISHOP_DIRS) & !friendly_occ;
            targets &= response_mask & pin_masks[idx as usize];
            for_each_bit(targets, |target| buffer.push(Move::new(from, target)));
        }

        let mut rooks = self.piece_bitboards[color.idx()][PieceKind::Rook.idx()];
        while rooks != 0 {
            let idx = rooks.trailing_zeros() as u8;
            rooks &= rooks - 1;
            let from = Square::from_index(idx);
            let mut targets =
                sliding_attacks_from(from, occupancy, &ROOK_DIRS) & !friendly_occ;
            targets &= response_mask & pin_masks[idx as usize];
            for_each_bit(targets, |target| buffer.push(Move::new(from, target)));
        }

        let mut queens = self.piece_bitboards[color.idx()][PieceKind::Queen.idx()];
        while queens != 0 {
            let idx = queens.trailing_zeros() as u8;
            queens &= queens - 1;
            let from = Square::from_index(idx);
            let mut targets = (sliding_attacks_from(from, occupancy, &BISHOP_DIRS)
                | sliding_attacks_from(from, occupancy, &ROOK_DIRS))
                & !friendly_occ;
            targets &= response_mask & pin_masks[idx as usize];
            for_each_bit(targets, |target| buffer.push(Move::new(from, target)));
        }
    }

    fn compute_pin_masks(
        &self,
        color: Color,
        king_square: Square,
        bishop_sliders: Bitboard,
        rook_sliders: Bitboard,
        pin_masks: &mut [Bitboard; 64],
    ) {
        self.detect_pins_in_dirs(
            color,
            king_square,
            bishop_sliders,
            &BISHOP_DIRS,
            pin_masks,
        );
        self.detect_pins_in_dirs(
            color,
            king_square,
            rook_sliders,
            &ROOK_DIRS,
            pin_masks,
        );
    }

    fn detect_pins_in_dirs(
        &self,
        color: Color,
        king_square: Square,
        enemy_sliders: Bitboard,
        dirs: &[(i8, i8)],
        pin_masks: &mut [Bitboard; 64],
    ) {
        let friendly_occ = self.occupancy_by_color[color.idx()];
        let enemy_occ = self.occupancy_by_color[color.opponent().idx()];
        for &(dr, df) in dirs {
            let mut current = king_square;
            let mut blocker: Option<Square> = None;
            while let Some(next) = current.offset(dr, df) {
                let mask = square_bitboard(next);
                if friendly_occ & mask != 0 {
                    if blocker.is_none() {
                        blocker = Some(next);
                    } else {
                        break;
                    }
                } else if enemy_occ & mask != 0 {
                    if enemy_sliders & mask != 0 {
                        if let Some(block_sq) = blocker {
                            let line_mask = (ray_mask_from(block_sq, dr, df)
                                | ray_mask_from(block_sq, -dr, -df))
                                & !square_bitboard(king_square);
                            pin_masks[block_sq.to_index()] = line_mask;
                        }
                    }
                    break;
                }
                current = next;
            }
        }
    }

    pub fn static_exchange_eval(&self, mv: Move) -> i32 {
        const MAX_DEPTH: usize = 32;
        let mut gain = [0_i32; MAX_DEPTH];
        let mut depth = 0;
        let target = mv.to;
        let target_mask = square_bitboard(target);
        let mut bitboards = self.piece_bitboards;
        let mut occ_by_color = self.occupancy_by_color;
        let mut occupancy = self.occupancy;
        let moving = match self.piece_at(mv.from) {
            Some(piece) => piece,
            None => return 0,
        };

        gain[0] = self
            .piece_at(target)
            .map(|p| piece_value(p.kind))
            .unwrap_or(0);

        if let Some(captured) = self.piece_at(target) {
            bitboards[captured.color.idx()][captured.kind.idx()] &= !target_mask;
            occ_by_color[captured.color.idx()] &= !target_mask;
            occupancy &= !target_mask;
        }

        let from_mask = square_bitboard(mv.from);
        bitboards[moving.color.idx()][moving.kind.idx()] &= !from_mask;
        occ_by_color[moving.color.idx()] &= !from_mask;
        occupancy &= !from_mask;

        bitboards[moving.color.idx()][moving.kind.idx()] |= target_mask;
        occ_by_color[moving.color.idx()] |= target_mask;
        occupancy |= target_mask;

        let mut current_value = piece_value(moving.kind);
        let mut attackers = attackers_to_with(target, occupancy, &bitboards);
        let mut side = moving.color.opponent();
        let mut target_owner = moving.color;
        let mut target_kind = moving.kind;

        while let Some((kind, sq)) =
            smallest_attacker_from(attackers, side, &bitboards)
        {
            depth += 1;
            gain[depth] = current_value - gain[depth - 1];
            if gain[depth].max(-gain[depth - 1]) < 0 {
                break;
            }

            let sq_mask = square_bitboard(sq);
            bitboards[side.idx()][kind.idx()] &= !sq_mask;
            occ_by_color[side.idx()] &= !sq_mask;
            occupancy &= !sq_mask;

            bitboards[target_owner.idx()][target_kind.idx()] &= !target_mask;
            occ_by_color[target_owner.idx()] &= !target_mask;

            bitboards[side.idx()][kind.idx()] |= target_mask;
            occ_by_color[side.idx()] |= target_mask;
            occupancy |= target_mask;

            current_value = piece_value(kind);
            target_owner = side;
            target_kind = kind;
            side = side.opponent();
            attackers = attackers_to_with(target, occupancy, &bitboards);
        }

        while depth > 0 {
            gain[depth - 1] = -gain[depth - 1].max(-gain[depth]);
            depth -= 1;
        }
        gain[0]
    }

    pub fn move_is_legal(&self, mv: Move) -> bool {
        let piece = match self.piece_at(mv.from) {
            Some(piece) => piece,
            None => return false,
        };

        if piece.color != self.active_color {
            return false;
        }

        let captured = self.piece_at(mv.to);
        if let Some(dest) = captured {
            if dest.color == piece.color {
                return false;
            }
        }

        self.move_is_legal_state(mv, piece, captured)
    }

    fn move_is_legal_state(
        &self,
        mv: Move,
        piece: Piece,
        captured: Option<Piece>,
    ) -> bool {
        let mut bitboards = self.piece_bitboards;
        let mut occ_by_color = self.occupancy_by_color;
        let mut occupancy = self.occupancy;

        let from_mask = square_bitboard(mv.from);
        let to_mask = square_bitboard(mv.to);

        if let Some(captured_piece) = captured {
            let idx = captured_piece.color.idx();
            let kind = captured_piece.kind.idx();
            bitboards[idx][kind] &= !to_mask;
            occ_by_color[idx] &= !to_mask;
            occupancy &= !to_mask;
        }

        let color_idx = piece.color.idx();
        let kind_idx = piece.kind.idx();
        bitboards[color_idx][kind_idx] &= !from_mask;
        occ_by_color[color_idx] &= !from_mask;
        occupancy &= !from_mask;

        bitboards[color_idx][kind_idx] |= to_mask;
        occ_by_color[color_idx] |= to_mask;
        occupancy |= to_mask;

        let king_bb = bitboards[color_idx][PieceKind::King.idx()];
        if king_bb == 0 {
            return false;
        }
        let king_square = Square::from_index(king_bb.trailing_zeros() as u8);
        !square_attacked_state(
            king_square,
            piece.color.opponent(),
            &bitboards,
            occupancy,
        )
    }

    pub(crate) fn make_move(&mut self, mv: Move) -> MoveUndo {
        let piece = self.piece_at(mv.from).expect("move must have a piece");
        self.make_move_internal(mv, piece)
    }

    fn make_move_internal(&mut self, mv: Move, piece: Piece) -> MoveUndo {
        let captured = self.piece_at(mv.to);
        let undo = MoveUndo {
            mv,
            moving_piece: piece,
            captured_piece: captured,
            prev_castling_rights: self.castling_rights,
            prev_en_passant: self.en_passant,
            prev_halfmove_clock: self.halfmove_clock,
            prev_fullmove_number: self.fullmove_number,
            prev_active_color: self.active_color,
        };

        self.update_castling_after_move(mv, piece, captured);

        self.set_piece(mv.from, None);

        let mut moved_piece = piece;
        if moved_piece.kind == PieceKind::Pawn
            && (mv.to.rank() == 0 || mv.to.rank() == 7)
        {
            moved_piece = Piece::new(moved_piece.color, PieceKind::Queen);
        }

        self.set_piece(mv.to, Some(moved_piece));
        self.set_en_passant(None);
        self.halfmove_clock =
            if moved_piece.kind == PieceKind::Pawn || captured.is_some() {
                0
            } else {
                self.halfmove_clock.saturating_add(1)
            };

        if self.active_color == Color::Black {
            self.fullmove_number += 1;
        }

        self.toggle_active_color();
        undo
    }

    pub(crate) fn unmake_move(&mut self, undo: MoveUndo) {
        self.set_active_color(undo.prev_active_color);
        self.fullmove_number = undo.prev_fullmove_number;
        self.halfmove_clock = undo.prev_halfmove_clock;
        self.set_castling_rights(undo.prev_castling_rights);
        self.set_en_passant(undo.prev_en_passant);

        self.set_piece(undo.mv.to, undo.captured_piece);
        self.set_piece(undo.mv.from, Some(undo.moving_piece));
    }

    pub(crate) fn make_null_move(&mut self) -> NullMoveUndo {
        let undo = NullMoveUndo {
            prev_en_passant: self.en_passant,
            prev_halfmove_clock: self.halfmove_clock,
            prev_fullmove_number: self.fullmove_number,
            prev_active_color: self.active_color,
        };
        self.set_en_passant(None);
        self.halfmove_clock = self.halfmove_clock.saturating_add(1);
        if self.active_color == Color::Black {
            self.fullmove_number = self.fullmove_number.saturating_add(1);
        }
        self.toggle_active_color();
        undo
    }

    pub(crate) fn unmake_null_move(&mut self, undo: NullMoveUndo) {
        self.set_active_color(undo.prev_active_color);
        self.fullmove_number = undo.prev_fullmove_number;
        self.halfmove_clock = undo.prev_halfmove_clock;
        self.set_en_passant(undo.prev_en_passant);
    }

    fn update_castling_after_move(
        &mut self,
        mv: Move,
        moving: Piece,
        captured: Option<Piece>,
    ) {
        let mut rights = self.castling_rights;
        if moving.kind == PieceKind::King {
            match moving.color {
                Color::White => {
                    rights.white_kingside = false;
                    rights.white_queenside = false;
                }
                Color::Black => {
                    rights.black_kingside = false;
                    rights.black_queenside = false;
                }
            }
        }

        if moving.kind == PieceKind::Rook {
            Self::disable_rook_castling(&mut rights, mv.from, moving.color);
        }

        if let Some(cp) = captured {
            if cp.kind == PieceKind::Rook {
                Self::disable_rook_castling(&mut rights, mv.to, cp.color);
            }
        }

        if rights != self.castling_rights {
            self.set_castling_rights(rights);
        }
    }

    fn disable_rook_castling(
        rights: &mut CastlingRights,
        square: Square,
        color: Color,
    ) {
        match (color, square.file(), square.rank()) {
            (Color::White, 0, 0) => rights.white_queenside = false,
            (Color::White, 7, 0) => rights.white_kingside = false,
            (Color::Black, 0, 7) => rights.black_queenside = false,
            (Color::Black, 7, 7) => rights.black_kingside = false,
            _ => {}
        }
    }

    pub fn is_in_check(&self, color: Color) -> bool {
        if let Some(king_square) = self.king_square(color) {
            self.square_attacked(king_square, color.opponent())
        } else {
            false
        }
    }

    fn king_square(&self, color: Color) -> Option<Square> {
        let bb = self.piece_bitboards[color.idx()][PieceKind::King.idx()];
        if bb != 0 {
            Some(Square::from_index(bb.trailing_zeros() as u8))
        } else {
            None
        }
    }

    fn square_attacked(&self, square: Square, by: Color) -> bool {
        square_attacked_state(square, by, &self.piece_bitboards, self.occupancy)
    }

    pub fn halfka(&self) -> Result<Vec<f32>> {
        let mut dense = vec![0_f32; 64 * (12 * 64)];
        let acc = &self.nnue[self.active_color.idx()];
        let _king_sq = acc.king_square.ok_or_else(|| E::msg("King is missing."))?;
        for &idx in &acc.active {
            dense[idx as usize] = 1.0;
        }
        Ok(dense)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starting_position_fen_matches_standard() {
        let board = Board::starting_position();
        assert_eq!(
            board.fen(),
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"
        );
    }

    #[test]
    fn empty_board_is_illegal_due_to_missing_kings() {
        let board = Board::empty();
        assert!(!board.is_legal());
    }

    #[test]
    fn pawn_on_back_rank_is_illegal() {
        let mut board = Board::empty();
        board.set_piece(
            Square::unchecked(4, 4),
            Some(Piece::new(Color::White, PieceKind::King)),
        );
        board.set_piece(
            Square::unchecked(4, 3),
            Some(Piece::new(Color::Black, PieceKind::King)),
        );
        board.set_piece(
            Square::unchecked(0, 0),
            Some(Piece::new(Color::White, PieceKind::Pawn)),
        );

        assert!(!board.is_legal());
    }

    #[test]
    fn castling_rights_require_rooks() {
        let mut board = Board::starting_position();
        board.set_piece(Square::unchecked(7, 7), None);
        assert!(!board.is_legal());
    }

    #[test]
    fn en_passant_square_must_be_on_third_or_sixth_rank() {
        let mut board = Board::starting_position();
        board.set_en_passant(Some(Square::unchecked(0, 0)));
        assert!(!board.is_legal());

        board.set_en_passant(Some(Square::unchecked(2, 0)));
        assert!(board.is_legal());
    }

    #[test]
    fn square_new_bounds_check() {
        assert!(Square::new(0, 0).is_some());
        assert!(Square::new(7, 7).is_some());
        assert!(Square::new(8, 0).is_none());
        assert!(Square::new(0, 8).is_none());
    }

    fn squares_from_bitboard(bb: Bitboard) -> Vec<String> {
        let mut out = Vec::new();
        for_each_bit(bb, |sq| out.push(sq.to_algebraic()));
        out.sort();
        out
    }

    #[test]
    fn knight_attack_mask_is_correct() {
        let sq = Square::from_algebraic("d4").expect("square");
        let attacks = knight_attacks(sq);
        let expected = vec![
            "b3".to_string(),
            "b5".to_string(),
            "c2".to_string(),
            "c6".to_string(),
            "e2".to_string(),
            "e6".to_string(),
            "f3".to_string(),
            "f5".to_string(),
        ];
        assert_eq!(squares_from_bitboard(attacks), expected);
    }

    #[test]
    fn sliding_attacks_respect_blockers() {
        let mut board = Board::empty();
        let bishop_sq = Square::from_algebraic("d4").unwrap();
        board.set_piece(bishop_sq, Some(Piece::new(Color::White, PieceKind::Bishop)));
        let blocker = Square::from_algebraic("f6").unwrap();
        board.set_piece(blocker, Some(Piece::new(Color::White, PieceKind::Pawn)));
        let occ = board.occupancy;
        let attacks = sliding_attacks_from(bishop_sq, occ, &BISHOP_DIRS);
        assert!(attacks & square_bitboard(blocker) != 0);
        let beyond = Square::from_algebraic("g7").unwrap();
        assert_eq!(attacks & square_bitboard(beyond), 0);
    }

    #[test]
    fn square_attacked_detects_sliders() {
        let mut board = Board::empty();
        let king_sq = Square::from_algebraic("e4").unwrap();
        board.set_piece(king_sq, Some(Piece::new(Color::White, PieceKind::King)));
        let rook_sq = Square::from_algebraic("e8").unwrap();
        board.set_piece(rook_sq, Some(Piece::new(Color::Black, PieceKind::Rook)));
        assert!(board.square_attacked(king_sq, Color::Black));
    }

    #[test]
    fn starting_fen_round_trip() {
        let start = Board::from_fen(
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
        )
        .expect("valid FEN");
        assert_eq!(
            start.fen(),
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"
        );
    }

    #[test]
    fn truncated_fen_defaults_missing_clocks() {
        let board = Board::from_fen("8/8/8/8/8/8/8/8 w - -")
            .expect("missing clocks should succeed");
        assert_eq!(board.halfmove_clock, 0);
        assert_eq!(board.fullmove_number, 1);

        let board = Board::from_fen("8/8/8/8/8/8/8/8 b - - 12")
            .expect("missing fullmove number should default to 1");
        assert_eq!(board.halfmove_clock, 12);
        assert_eq!(board.fullmove_number, 1);
    }

    #[test]
    fn invalid_fen_reports_error() {
        assert!(Board::from_fen("invalid fen string").is_err());
        assert!(Board::from_fen("8/8/8/8/8/8/8/8 w - - 0 0").is_err());
    }

    #[test]
    fn starting_position_has_twenty_legal_moves() {
        let mut board = Board::starting_position();
        let mut moves = MoveList::new();
        board.legal_moves_into(&mut moves);
        assert_eq!(moves.len(), 20);
    }

    #[test]
    fn pinned_piece_cannot_move() {
        let mut board = Board::empty();
        board.set_active_color(Color::White);
        board.set_piece(
            Square::unchecked(7, 4),
            Some(Piece::new(Color::White, PieceKind::King)),
        );
        board.set_piece(
            Square::unchecked(0, 0),
            Some(Piece::new(Color::Black, PieceKind::King)),
        );
        board.set_piece(
            Square::unchecked(6, 4),
            Some(Piece::new(Color::White, PieceKind::Rook)),
        );
        board.set_piece(
            Square::unchecked(0, 4),
            Some(Piece::new(Color::Black, PieceKind::Rook)),
        );

        let mut moves = MoveList::new();
        board.legal_moves_into(&mut moves);
        let illegal = Move::new(Square::unchecked(6, 4), Square::unchecked(6, 5));
        assert!(
            !moves.contains(&illegal),
            "Pinned rook should not be allowed to move"
        );
    }

    #[test]
    fn halfka_starting_position() {
        let board = Board::starting_position();
        let features = board.halfka().expect("halfka should succeed");

        assert_eq!(features.len(), 64 * 12 * 64);
        let ones = features.iter().filter(|&&v| v == 1.0).count();
        assert_eq!(
            ones, 32,
            "starting position should have 32 feature activations"
        );

        let king_sq = (0_u8, 4_u8); // e1
        let idx = |piece_sq: (u8, u8), kind: PieceKind, friendly: bool| -> usize {
            let king_index = king_sq.0 as usize * 8 + king_sq.1 as usize;
            let plane_offset = king_index * (12 * 64);
            let square_index = piece_sq.0 as usize * 8 + piece_sq.1 as usize;
            let kind_index = kind as usize + if friendly { 0 } else { 6 };
            plane_offset + kind_index * 64 + square_index
        };

        // White pawn on a2
        assert_eq!(features[idx((1, 0), PieceKind::Pawn, true)], 1.0);
        // Black pawn on a7
        assert_eq!(features[idx((6, 0), PieceKind::Pawn, false)], 1.0);
        // White king on e1
        assert_eq!(features[idx((0, 4), PieceKind::King, true)], 1.0);
        // Black king on e8
        assert_eq!(features[idx((7, 4), PieceKind::King, false)], 1.0);
    }

    #[test]
    fn promotion_move_to_uci_includes_suffix() {
        let mv =
            Move::with_promotion(Square::unchecked(6, 0), Square::unchecked(7, 0));
        assert_eq!(mv.to_uci(), "a7a8q");
        let capture =
            Move::with_promotion(Square::unchecked(1, 5), Square::unchecked(0, 4));
        assert_eq!(capture.to_uci(), "f2e1q");
    }

    #[test]
    fn halfka_complex_position_1() {
        let mut board = Board::empty();
        board.set_active_color(Color::White);
        board.set_piece(
            Square::unchecked(0, 0),
            Some(Piece::new(Color::White, PieceKind::King)),
        );
        board.set_piece(
            Square::unchecked(0, 1),
            Some(Piece::new(Color::White, PieceKind::Knight)),
        );
        board.set_piece(
            Square::unchecked(7, 7),
            Some(Piece::new(Color::Black, PieceKind::Rook)),
        );

        let features = board.halfka().expect("halfka should succeed");
        let king_sq = (0_u8, 0_u8); // a1
        let idx = |piece_sq: (u8, u8), kind: PieceKind, friendly: bool| -> usize {
            let king_index = king_sq.0 as usize * 8 + king_sq.1 as usize;
            let plane_offset = king_index * (12 * 64);
            let square_index = piece_sq.0 as usize * 8 + piece_sq.1 as usize;
            let kind_index = kind as usize + if friendly { 0 } else { 6 };
            plane_offset + kind_index * 64 + square_index
        };

        assert_eq!(features[idx((0, 0), PieceKind::King, true)], 1.0);
        assert_eq!(features[idx((0, 1), PieceKind::Knight, true)], 1.0);
        assert_eq!(features[idx((7, 7), PieceKind::Rook, false)], 1.0);

        let ones = features.iter().filter(|&&v| v == 1.0).count();
        assert_eq!(ones, 3);
    }

    #[test]
    fn halfka_complex_position_2() {
        let mut board = Board::empty();
        board.set_active_color(Color::Black);
        board.set_piece(
            Square::unchecked(7, 7),
            Some(Piece::new(Color::Black, PieceKind::King)),
        );
        board.set_piece(
            Square::unchecked(6, 7),
            Some(Piece::new(Color::Black, PieceKind::Pawn)),
        );
        board.set_piece(
            Square::unchecked(0, 0),
            Some(Piece::new(Color::White, PieceKind::Bishop)),
        );

        let features = board.halfka().expect("halfka should succeed");

        let king_sq = (0_u8, 7_u8); // mirrored h8 after flip
        let idx = |piece_sq: (u8, u8), kind: PieceKind, friendly: bool| -> usize {
            let king_index = king_sq.0 as usize * 8 + king_sq.1 as usize;
            let plane_offset = king_index * (12 * 64);
            let square_index = piece_sq.0 as usize * 8 + piece_sq.1 as usize;
            let kind_index = kind as usize + if friendly { 0 } else { 6 };
            plane_offset + kind_index * 64 + square_index
        };

        // Black king becomes white king on h1 after flip.
        assert_eq!(features[idx((0, 7), PieceKind::King, true)], 1.0);
        // Black pawn becomes white pawn on h2 after flip.
        assert_eq!(features[idx((1, 7), PieceKind::Pawn, true)], 1.0);
        // White bishop becomes black bishop on a8 after flip.
        assert_eq!(features[idx((7, 0), PieceKind::Bishop, false)], 1.0);

        let ones = features.iter().filter(|&&v| v == 1.0).count();
        assert_eq!(ones, 3);
    }

    #[test]
    fn halfka_complex_position_3() {
        let mut board = Board::empty();
        board.set_active_color(Color::White);
        board.set_piece(
            Square::unchecked(7, 7),
            Some(Piece::new(Color::Black, PieceKind::King)),
        );

        assert!(
            board.halfka().is_err(),
            "missing active-color king should error"
        );
    }
}
