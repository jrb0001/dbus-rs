use super::utils::{ArcMap, Annotations, Introspect};
use super::{MethodType, MethodInfo, MethodResult, MethodErr, DataType, Property, Method, Signal};
use std::sync::{Arc, Mutex};
use {Member, Message, Path, MessageType, Connection, ConnectionItem, Error, arg};
use Interface as IfaceName;
use std::fmt;
use std::ffi::{CStr, CString};
use super::leaves::prop_append_dict;

fn introspect_map<I: fmt::Display, T: Introspect>
    (h: &ArcMap<I, T>, indent: &str) -> String {

    h.iter().fold("".into(), |a, (k, v)| {
        let (name, params, contents) = (v.xml_name(), v.xml_params(), v.xml_contents());
        format!("{}{}<{} name=\"{}\"{}{}>\n",
            a, indent, name, &*k, params, if contents.len() > 0 {
                format!(">\n{}{}</{}", contents, indent, name)
            }
            else { format!("/") }
        )
    })
}

#[derive(Debug)]
/// Represents a D-Bus interface.
pub struct Interface<M: MethodType<D>, D: DataType> {
    name: Arc<IfaceName<'static>>,
    methods: ArcMap<Member<'static>, Method<M, D>>,
    signals: ArcMap<Member<'static>, Signal<D>>,
    properties: ArcMap<String, Property<M, D>>,
    anns: Annotations,
    data: D::Interface,
}

impl<M: MethodType<D>, D: DataType> Interface<M, D> {
    /// Builder function that adds a method to the interface.
    pub fn add_m<I: Into<Arc<Method<M, D>>>>(mut self, m: I) -> Self {
        let m = m.into();
        self.methods.insert(m.get_name().clone(), m);
        self
    }

    /// Builder function that adds a signal to the interface.
    pub fn add_s<I: Into<Arc<Signal<D>>>>(mut self, s: I) -> Self {
        let m = s.into();
        self.signals.insert(m.get_name().clone(), m);
        self
    }

    /// Builder function that adds a property to the interface.
    pub fn add_p<I: Into<Arc<Property<M, D>>>>(mut self, p: I) -> Self {
        let m = p.into();
        self.properties.insert(m.get_name().to_owned(), m);
        self
    }

    /// Builder function that adds an annotation to this interface.
    pub fn annotate<N: Into<String>, V: Into<String>>(mut self, name: N, value: V) -> Self {
        self.anns.insert(name, value); self
    }

    /// Builder function that adds an annotation that this entity is deprecated.
    pub fn deprecated(self) -> Self { self.annotate("org.freedesktop.DBus.Deprecated", "true") }

    /// Get interface name
    pub fn get_name(&self) -> &IfaceName<'static> { &self.name }

    /// Get associated data
    pub fn get_data(&self) -> &D::Interface { &self.data }

}

impl<M: MethodType<D>, D: DataType> Introspect for Interface<M, D> {
    fn xml_name(&self) -> &'static str { "interface" }
    fn xml_params(&self) -> String { String::new() }
    fn xml_contents(&self) -> String {
        format!("{}{}{}{}",
            introspect_map(&self.methods, "    "),
            introspect_map(&self.properties, "    "),
            introspect_map(&self.signals, "    "),
            self.anns.introspect("    "))
    }
}


pub fn new_interface<M: MethodType<D>, D: DataType>(t: IfaceName<'static>, d: D::Interface) -> Interface<M, D> {
    Interface { name: Arc::new(t), methods: ArcMap::new(), signals: ArcMap::new(),
        properties: ArcMap::new(), anns: Annotations::new(), data: d
    }
}


#[derive(Debug)]
/// Cache of built-in interfaces, in order to save memory when many object paths implement the same interface(s).
pub struct IfaceCache<M: MethodType<D>, D: DataType>(Mutex<ArcMap<IfaceName<'static>, Interface<M, D>>>);

impl<M: MethodType<D>, D: DataType> IfaceCache<M, D> {
    pub fn get<S: Into<IfaceName<'static>> + Clone, F>(&self, s: S, f: F) -> Arc<Interface<M, D>>
        where F: FnOnce(Interface<M, D>) -> Interface<M, D> {
        let s2 = s.clone().into();
        let mut m = self.0.lock().unwrap();
        m.entry(s2).or_insert_with(|| {
            let i = new_interface(s.into(), Default::default());
            Arc::new(f(i))
        }).clone()
    }

    pub fn new() -> Arc<Self> { Arc::new(IfaceCache(Mutex::new(ArcMap::new()))) }
}

#[derive(Debug)]
/// A D-Bus Object Path.
pub struct ObjectPath<M: MethodType<D>, D: DataType> {
    name: Arc<Path<'static>>,
    ifaces: ArcMap<Arc<IfaceName<'static>>, Interface<M, D>>,
    ifacecache: Arc<IfaceCache<M, D>>,
    data: D::ObjectPath,
}

impl<M: MethodType<D>, D: DataType> ObjectPath<M, D> {
    /// Builder function that adds a interface to the object path.
    pub fn add<I: Into<Arc<Interface<M, D>>>>(mut self, s: I) -> Self {
        let m = s.into();
        if !m.properties.is_empty() { self.add_property_handler(); }
        self.ifaces.insert(m.name.clone(), m);
        self
    }

    /// Get property name
    pub fn get_name(&self) -> &Path<'static> { &self.name }

    /// Get associated data
    pub fn get_data(&self) -> &D::ObjectPath { &self.data }

    /// Adds introspection support for this object path.
    pub fn introspectable(self) -> Self {
        let z = self.ifacecache.get("org.freedesktop.DBus.Introspectable", |i| {
            i.add_m(
                super::leaves::new_method("Introspect".into(), Default::default(), M::make_method(Box::new(|m| {
                    Ok(vec!(m.msg.method_return().append1(m.path.introspect(m.tree))))
                }))).out_arg(("xml_data", "s"))
             )
        });
        self.add(z)
    }

    fn introspect(&self, tree: &Tree<M, D>) -> String {
        let ifacestr = introspect_map(&self.ifaces, "  ");
        let olen = self.name.len()+1;
        let childstr = tree.children(self, true).iter().fold("".to_string(), |na, n|
            format!("{}  <node name=\"{}\"/>\n", na, &n.name[olen..])
        );

        let nodestr = format!(r##"<!DOCTYPE node PUBLIC "-//freedesktop//DTD D-BUS Object Introspection 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/introspect.dtd">
<node name="{}">
{}{}</node>"##, self.name, ifacestr, childstr);
        nodestr
    }

    fn add_property_handler(&mut self) {
        use arg::{Variant, Dict};
        let ifname = IfaceName::from("org.freedesktop.DBus.Properties");
        if self.ifaces.contains_key(&ifname) { return };
        let z = self.ifacecache.get(ifname, |i| {
            i.add_m(super::leaves::new_method("Get".into(), Default::default(),
                M::make_method(Box::new(|m| m.path.prop_get(m))))
                .inarg::<&str,_>("interface_name")
                .inarg::<&str,_>("property_name")
                .outarg::<Variant<()>,_>("value"))
            .add_m(super::leaves::new_method("GetAll".into(), Default::default(),
                M::make_method(Box::new(|m| m.path.prop_get_all(m))))
                .inarg::<&str,_>("interface_name")
                .outarg::<Dict<&str, Variant<()>, ()>,_>("props"))
            .add_m(super::leaves::new_method("Set".into(), Default::default(),
                M::make_method(Box::new(|m| m.path.prop_set(m))))
                .inarg::<&str,_>("interface_name")
                .inarg::<&str,_>("property_name")
                .inarg::<Variant<bool>,_>("value"))
        });
        self.ifaces.insert(z.name.clone(), z);
    }

    fn get_iface<'a>(&'a self, i: Option<&'a CStr>) -> Result<&Arc<Interface<M, D>>, MethodErr> {
        let iface_name = try!(i.ok_or_else(|| MethodErr::invalid_arg(&0)));
        let j = try!(IfaceName::from_slice(iface_name.to_bytes_with_nul()).map_err(|e| MethodErr::invalid_arg(&e)));
        self.ifaces.get(&j).ok_or_else(|| MethodErr::no_interface(&j))
    }

    fn prop_get(&self, m: &MethodInfo<M, D>) -> MethodResult {
        let (iname, p) = m.msg.get2();
        let iface = try!(self.get_iface(iname));
        let prop_name: &str = try!(p.ok_or_else(|| MethodErr::invalid_arg(&1)));
        let prop: &Property<M, D> = try!(iface.properties.get(&String::from(prop_name))
            .ok_or_else(|| MethodErr::no_property(&prop_name)));
        try!(prop.can_get());
        let mut mret = m.msg.method_return();
        {
            let mut iter = arg::IterAppend::new(&mut mret); 
            let pinfo = m.to_prop_info(iface, prop);
            try!(prop.get_as_variant(&mut iter, &pinfo));
        }
        Ok(vec!(mret))
    }

    fn prop_get_all(&self, m: &MethodInfo<M, D>) -> MethodResult {
        let iface = try!(self.get_iface(m.msg.get1()));
        let mut mret = m.msg.method_return(); 
        try!(prop_append_dict(&mut arg::IterAppend::new(&mut mret), 
            iface.properties.values().map(|v| &**v), m));
        Ok(vec!(mret))
    }


    fn prop_set(&self, m: &MethodInfo<M, D>) -> MethodResult {
        let (iname, p) = m.msg.get2();
        let iface = try!(self.get_iface(iname));
        let prop_name: &str = try!(p.ok_or_else(|| MethodErr::invalid_arg(&1)));
        let prop: &Property<M, D> = try!(iface.properties.get(&String::from(prop_name))
            .ok_or_else(|| MethodErr::no_property(&prop_name)));

        let mut iter = arg::Iter::new(m.msg);
        iter.next(); iter.next();
        let mut iter2 = iter;
        try!(prop.can_set(Some(iter)));

        let pinfo = m.to_prop_info(iface, prop);
        let mut r: Vec<Message> = try!(prop.set_as_variant(&mut iter2, &pinfo)).into_iter().collect();
        r.push(m.msg.method_return());
        Ok(r)

    }

    fn handle(&self, m: &Message, t: &Tree<M, D>) -> MethodResult {
        let i = try!(m.interface().and_then(|i| self.ifaces.get(&i)).ok_or_else(|| MethodErr::no_interface(&"")));
        let me = try!(m.member().and_then(|me| i.methods.get(&me)).ok_or_else(|| MethodErr::no_method(&"")));
        let minfo = MethodInfo { msg: m, tree: t, path: self, iface: i, method: me };
        me.call(&minfo)
    }

}


pub fn new_objectpath<M: MethodType<D>, D: DataType>(n: Path<'static>, d: D::ObjectPath, cache: Arc<IfaceCache<M, D>>)
    -> ObjectPath<M, D> {
    ObjectPath { name: Arc::new(n), data: d, ifaces: ArcMap::new(), ifacecache: cache }
}


/// A collection of object paths.
#[derive(Debug, Default)]
pub struct Tree<M: MethodType<D>, D: DataType> {
    paths: ArcMap<Arc<Path<'static>>, ObjectPath<M, D>>
}

impl<M: MethodType<D>, D: DataType> Tree<M, D> {
    /// Builder function that adds am object path to this tree.
    ///
    /// Note: This does not register a path with the connection, so if the tree is currently registered,
    /// you might want to call Connection::register_object_path to add the path manually.
    pub fn add<I: Into<Arc<ObjectPath<M, D>>>>(mut self, s: I) -> Self {
        let m = s.into();
        self.paths.insert(m.name.clone(), m);
        self
    }


    /// Remove a object path from the Tree. Returns the object path removed, or None if not found.
    ///
    /// Note: This does not unregister a path with the connection, so if the tree is currently registered,
    /// you might want to call Connection::unregister_object_path to remove the path manually.
    pub fn remove(&mut self, p: &Path<'static>) -> Option<Arc<ObjectPath<M, D>>> {
        // There is no real reason p needs to have a static lifetime; but
        // the borrow checker doesn't agree. :-(
        self.paths.remove(p)
    }

    /// Registers or unregisters all object paths in the tree.
    pub fn set_registered(&self, c: &Connection, b: bool) -> Result<(), Error> {
        let mut regd_paths = Vec::new();
        for p in self.paths.keys() {
            if b {
                match c.register_object_path(p) {
                    Ok(()) => regd_paths.push(p.clone()),
                    Err(e) => {
                        while let Some(rp) = regd_paths.pop() {
                            c.unregister_object_path(&rp);
                        }
                        return Err(e)
                    }
                }
            } else {
                c.unregister_object_path(p);
            }
        }
        Ok(())
    }

    /// This method takes an `ConnectionItem` iterator (you get it from `Connection::iter()`)
    /// and handles all matching items. Non-matching items (e g signals) are passed through.
    pub fn run<'a, I: Iterator<Item=ConnectionItem>>(&'a self, c: &'a Connection, i: I) -> TreeServer<'a, I, M, D> {
        TreeServer { iter: i, tree: &self, conn: c }
    }

    /// Handles a message.
    ///
    /// Will return None in case the object path was not
    /// found in this tree, or otherwise a list of messages to be sent back.
    pub fn handle(&self, m: &Message) -> Option<Vec<Message>> {
        if m.msg_type() != MessageType::MethodCall { None }
        else { m.path().and_then(|p| self.paths.get(&p).map(|s| s.handle(m, &self)
            .unwrap_or_else(|e| vec!(m.error(&e.errorname(), &CString::new(e.description()).unwrap()))))) }
    }


    fn children(&self, o: &ObjectPath<M, D>, direct_only: bool) -> Vec<&ObjectPath<M, D>> {
        let parent: &str = &o.name;
        let plen = parent.len()+1;
        self.paths.values().filter_map(|v| {
            let k: &str = &v.name;
            if !k.starts_with(parent) || k.len() <= plen || &k[plen-1..plen] != "/" {None} else {
                let child = &k[plen..];
                if direct_only && child.contains("/") {None} else {Some(&**v)}
            }
        }).collect()
    }

}

/// An iterator adapter that handles incoming method calls.
///
/// Method calls that match an object path in the tree are handled and consumed by this
/// iterator. Other messages are passed through.
pub struct TreeServer<'a, I, M: MethodType<D> + 'a, D: DataType + 'a> {
    iter: I,
    conn: &'a Connection,
    tree: &'a Tree<M, D>,
}

impl<'a, I: Iterator<Item=ConnectionItem>, M: 'a + MethodType<D>, D: DataType + 'a> Iterator for TreeServer<'a, I, M, D> {
    type Item = ConnectionItem;

    fn next(&mut self) -> Option<ConnectionItem> {
        loop {
            let n = self.iter.next();
            if let &Some(ConnectionItem::MethodCall(ref msg)) = &n {
                if let Some(v) = self.tree.handle(&msg) {
                    // Probably the wisest is to ignore any send errors here -
                    // maybe the remote has disconnected during our processing.
                    for m in v { let _ = self.conn.send(m); };
                    continue;
                }
            }
            return n;
        }
    }
}




#[test]
fn test_introspection() {
    let f = super::Factory::new_fn::<()>();
    let t = f.object_path("/echo", ()).introspectable()
        .add(f.interface("com.example.echo", ())
            .add_m(f.method("Echo", (), |_| unimplemented!()).in_arg(("request", "s")).out_arg(("reply", "s")))
            .add_p(f.property::<i32,_>("EchoCount", ()))
            .add_s(f.signal("Echoed", ()).arg(("data", "s")).deprecated())
    );

    let actual_result = t.introspect(&f.tree().add(f.object_path("/echo/subpath", ())));
    println!("\n=== Introspection XML start ===\n{}\n=== Introspection XML end ===", actual_result);

    let expected_result = r##"<!DOCTYPE node PUBLIC "-//freedesktop//DTD D-BUS Object Introspection 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/introspect.dtd">
<node name="/echo">
  <interface name="com.example.echo">
    <method name="Echo">
      <arg name="request" type="s" direction="in"/>
      <arg name="reply" type="s" direction="out"/>
    </method>
    <property name="EchoCount" type="i" access="read"/>
    <signal name="Echoed">
      <arg name="data" type="s"/>
      <annotation name="org.freedesktop.DBus.Deprecated" value="true"/>
    </signal>
  </interface>
  <interface name="org.freedesktop.DBus.Introspectable">
    <method name="Introspect">
      <arg name="xml_data" type="s" direction="out"/>
    </method>
  </interface>
  <interface name="org.freedesktop.DBus.Properties">
    <method name="Get">
      <arg name="interface_name" type="s" direction="in"/>
      <arg name="property_name" type="s" direction="in"/>
      <arg name="value" type="v" direction="out"/>
    </method>
    <method name="GetAll">
      <arg name="interface_name" type="s" direction="in"/>
      <arg name="props" type="a{sv}" direction="out"/>
    </method>
    <method name="Set">
      <arg name="interface_name" type="s" direction="in"/>
      <arg name="property_name" type="s" direction="in"/>
      <arg name="value" type="v" direction="in"/>
    </method>
  </interface>
  <node name="subpath"/>
</node>"##;
 
    assert_eq!(expected_result, actual_result);   
}

