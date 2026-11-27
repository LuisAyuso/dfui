//! The cities shown, by region: the capital first, then the others in its
//! region. Regions are named as in the GISCO NUTS 2 data.

pub struct City {
    pub name: &'static str,
    pub region: &'static str,
    /// The region's capital, shown on the whole country's map.
    pub capital: bool,
    pub lat: f64,
    pub lon: f64,
    /// INE municipality code, AEMET's id; empty outside Spain.
    pub aemet: &'static str,
}

const fn capital(
    region: &'static str,
    name: &'static str,
    lat: f64,
    lon: f64,
    aemet: &'static str,
) -> City {
    City {
        name,
        region,
        capital: true,
        lat,
        lon,
        aemet,
    }
}

const fn town(
    region: &'static str,
    name: &'static str,
    lat: f64,
    lon: f64,
    aemet: &'static str,
) -> City {
    City {
        name,
        region,
        capital: false,
        lat,
        lon,
        aemet,
    }
}

const AND: &str = "Andalucía";
const ARA: &str = "Aragón";
const AST: &str = "Principado de Asturias";
const BAL: &str = "Illes Balears";
const CAN: &str = "Canarias";
const CTB: &str = "Cantabria";
const CLM: &str = "Castilla-La Mancha";
const CYL: &str = "Castilla y León";
const CAT: &str = "Cataluña";
const CEU: &str = "Ciudad de Ceuta";
const MEL: &str = "Ciudad de Melilla";
const VAL: &str = "Comunitat Valenciana";
const EXT: &str = "Extremadura";
const GAL: &str = "Galicia";
const MAD: &str = "Comunidad de Madrid";
const MUR: &str = "Región de Murcia";
const NAV: &str = "Comunidad Foral de Navarra";
const PVA: &str = "País Vasco";
const RIO: &str = "La Rioja";

pub const SPAIN: &[City] = &[
    capital(AND, "Sevilla", 37.3891, -5.9845, "41091"),
    town(AND, "Almería", 36.8340, -2.4637, "04013"),
    town(AND, "Cádiz", 36.5271, -6.2886, "11012"),
    town(AND, "Córdoba", 37.8882, -4.7794, "14021"),
    town(AND, "Granada", 37.1773, -3.5986, "18087"),
    town(AND, "Huelva", 37.2614, -6.9447, "21041"),
    town(AND, "Jaén", 37.7796, -3.7849, "23050"),
    town(AND, "Málaga", 36.7213, -4.4214, "29067"),
    capital(ARA, "Zaragoza", 41.6488, -0.8891, "50297"),
    town(ARA, "Huesca", 42.1401, -0.4089, "22125"),
    town(ARA, "Teruel", 40.3456, -1.1065, "44216"),
    capital(AST, "Oviedo", 43.3614, -5.8494, "33044"),
    town(AST, "Gijón", 43.5322, -5.6611, "33024"),
    town(AST, "Avilés", 43.5547, -5.9248, "33004"),
    capital(BAL, "Palma", 39.5696, 2.6502, "07040"),
    town(BAL, "Eivissa", 38.9067, 1.4206, "07026"),
    town(BAL, "Maó", 39.8885, 4.2658, "07032"),
    capital(CAN, "Las Palmas", 28.1235, -15.4363, "35016"),
    town(CAN, "Santa Cruz de Tenerife", 28.4636, -16.2518, "38038"),
    town(CAN, "Arrecife", 28.9630, -13.5477, "35004"),
    town(CAN, "Puerto del Rosario", 28.5004, -13.8627, "35017"),
    town(CAN, "Santa Cruz de La Palma", 28.6835, -17.7642, "38037"),
    capital(CTB, "Santander", 43.4623, -3.8100, "39075"),
    town(CTB, "Torrelavega", 43.3494, -4.0479, "39087"),
    capital(CLM, "Toledo", 39.8628, -4.0273, "45168"),
    town(CLM, "Albacete", 38.9943, -1.8585, "02003"),
    town(CLM, "Ciudad Real", 38.9848, -3.9274, "13034"),
    town(CLM, "Cuenca", 40.0704, -2.1374, "16078"),
    town(CLM, "Guadalajara", 40.6329, -3.1669, "19130"),
    capital(CYL, "Valladolid", 41.6523, -4.7245, "47186"),
    town(CYL, "Ávila", 40.6566, -4.6818, "05019"),
    town(CYL, "Burgos", 42.3439, -3.6969, "09059"),
    town(CYL, "León", 42.5987, -5.5671, "24089"),
    town(CYL, "Palencia", 42.0095, -4.5288, "34120"),
    town(CYL, "Salamanca", 40.9701, -5.6635, "37274"),
    town(CYL, "Segovia", 40.9429, -4.1088, "40194"),
    town(CYL, "Soria", 41.7666, -2.4790, "42173"),
    town(CYL, "Zamora", 41.5035, -5.7446, "49275"),
    capital(CAT, "Barcelona", 41.3874, 2.1686, "08019"),
    town(CAT, "Girona", 41.9794, 2.8214, "17079"),
    town(CAT, "Lleida", 41.6176, 0.6200, "25120"),
    town(CAT, "Tarragona", 41.1189, 1.2445, "43148"),
    capital(CEU, "Ceuta", 35.8894, -5.3213, "51001"),
    capital(MEL, "Melilla", 35.2923, -2.9381, "52001"),
    capital(VAL, "Valencia", 39.4699, -0.3763, "46250"),
    town(VAL, "Alicante", 38.3452, -0.4810, "03014"),
    town(VAL, "Castellón de la Plana", 39.9864, -0.0513, "12040"),
    town(VAL, "Elche", 38.2669, -0.6983, "03065"),
    capital(EXT, "Mérida", 38.9161, -6.3437, "06083"),
    town(EXT, "Badajoz", 38.8794, -6.9707, "06015"),
    town(EXT, "Cáceres", 39.4753, -6.3724, "10037"),
    town(EXT, "Plasencia", 40.0303, -6.0884, "10148"),
    capital(GAL, "Santiago de Compostela", 42.8782, -8.5448, "15078"),
    town(GAL, "A Coruña", 43.3623, -8.4115, "15030"),
    town(GAL, "Lugo", 43.0097, -7.5568, "27028"),
    town(GAL, "Ourense", 42.3358, -7.8639, "32054"),
    town(GAL, "Pontevedra", 42.4310, -8.6444, "36038"),
    town(GAL, "Vigo", 42.2406, -8.7207, "36057"),
    capital(MAD, "Madrid", 40.4168, -3.7038, "28079"),
    town(MAD, "Alcalá de Henares", 40.4818, -3.3643, "28005"),
    town(MAD, "Aranjuez", 40.0311, -3.6025, "28013"),
    capital(MUR, "Murcia", 37.9922, -1.1307, "30030"),
    town(MUR, "Cartagena", 37.6257, -0.9966, "30016"),
    town(MUR, "Lorca", 37.6771, -1.7006, "30024"),
    capital(NAV, "Pamplona", 42.8125, -1.6458, "31201"),
    town(NAV, "Tudela", 42.0617, -1.6045, "31232"),
    capital(PVA, "Vitoria-Gasteiz", 42.8467, -2.6716, "01059"),
    town(PVA, "Bilbao", 43.2630, -2.9350, "48020"),
    town(PVA, "Donostia", 43.3183, -1.9812, "20069"),
    capital(RIO, "Logroño", 42.4627, -2.4450, "26089"),
    town(RIO, "Calahorra", 42.3050, -1.9653, "26036"),
];

const W: &str = "Wien";
const NOE: &str = "Niederösterreich";
const OOE: &str = "Oberösterreich";
const SBG: &str = "Salzburg";
const T: &str = "Tirol";
const V: &str = "Vorarlberg";
const K: &str = "Kärnten";
const ST: &str = "Steiermark";
const B: &str = "Burgenland";

pub const AUSTRIA: &[City] = &[
    capital(W, "Wien", 48.2082, 16.3738, ""),
    capital(NOE, "St. Pölten", 48.2047, 15.6256, ""),
    town(NOE, "Wiener Neustadt", 47.8151, 16.2465, ""),
    town(NOE, "Krems", 48.4100, 15.6142, ""),
    town(NOE, "Amstetten", 48.1229, 14.8720, ""),
    town(NOE, "Zwettl", 48.6033, 15.1689, ""),
    capital(OOE, "Linz", 48.3069, 14.2858, ""),
    town(OOE, "Wels", 48.1575, 14.0289, ""),
    town(OOE, "Steyr", 48.0427, 14.4213, ""),
    town(OOE, "Braunau", 48.2563, 13.0349, ""),
    town(OOE, "Freistadt", 48.5115, 14.5045, ""),
    town(OOE, "Bad Ischl", 47.7116, 13.6196, ""),
    capital(SBG, "Salzburg", 47.8095, 13.0550, ""),
    town(SBG, "Zell am See", 47.3236, 12.7966, ""),
    town(SBG, "Bischofshofen", 47.4172, 13.2196, ""),
    town(SBG, "Tamsweg", 47.1281, 13.8106, ""),
    capital(T, "Innsbruck", 47.2692, 11.4041, ""),
    town(T, "Kufstein", 47.5833, 12.1667, ""),
    town(T, "Kitzbühel", 47.4464, 12.3922, ""),
    town(T, "Landeck", 47.1397, 10.5658, ""),
    town(T, "Reutte", 47.4833, 10.7167, ""),
    town(T, "Lienz", 46.8297, 12.7697, ""),
    capital(V, "Bregenz", 47.5031, 9.7471, ""),
    town(V, "Dornbirn", 47.4125, 9.7417, ""),
    town(V, "Feldkirch", 47.2333, 9.6000, ""),
    town(V, "Bludenz", 47.1547, 9.8222, ""),
    capital(K, "Klagenfurt", 46.6247, 14.3053, ""),
    town(K, "Villach", 46.6111, 13.8558, ""),
    town(K, "Spittal an der Drau", 46.8000, 13.5000, ""),
    town(K, "Wolfsberg", 46.8417, 14.8417, ""),
    capital(ST, "Graz", 47.0707, 15.4395, ""),
    town(ST, "Leoben", 47.3765, 15.0915, ""),
    town(ST, "Liezen", 47.5667, 14.2333, ""),
    town(ST, "Leibnitz", 46.7833, 15.5333, ""),
    town(ST, "Hartberg", 47.2806, 15.9700, ""),
    town(ST, "Schladming", 47.3929, 13.6872, ""),
    capital(B, "Eisenstadt", 47.8457, 16.5233, ""),
    town(B, "Neusiedl am See", 47.9486, 16.8431, ""),
    town(B, "Oberwart", 47.2878, 16.2031, ""),
    town(B, "Güssing", 47.0583, 16.3236, ""),
];
